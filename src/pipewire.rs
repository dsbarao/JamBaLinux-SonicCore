//! Persistent PipeWire backend for the host-side Game equalizer.
//!
//! The backend deliberately never discovers or opens a HID device. It keeps a
//! single filter-chain alive, mutates all ten DSP controls in one PipeWire
//! `Props` update, and only routes playback streams whose current destination
//! is the confirmed Quantum Game sink. Chat playback and every capture source
//! stay outside the chain.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::equalizer::{BANDS_HZ, Band, EqualizerProfile};
use crate::spatial::graph::{
    SPATIAL_OUTPUT_NODE, SPATIAL_SINK_NODE, TARGET_EQUALIZER_SINK, spatial_mix,
};

const UNIT_NAME: &str = "jambalinux-soniccore-equalizer.service";
const SPATIAL_UNIT_NAME: &str = "jambalinux-soniccore-spatial.service";
const CONFIG_NAME: &str = "pipewire-game-equalizer.conf";
const STATE_NAME: &str = "pipewire-game-equalizer.json";
const LOCK_NAME: &str = "pipewire-game-equalizer.lock";
const VIRTUAL_SINK: &str = "jambalinux-soniccore-game-equalizer";
const OUTPUT_NODE: &str = "jambalinux-soniccore-game-equalizer-output";
const QUANTUM_ALSA_COMPONENTS: &str = "USB0ecb:2069";
const ROUTE_INTERVAL: Duration = Duration::from_millis(500);
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
// A gain change is split into steps of at most RAMP_MAX_STEP_DB, one per audio
// cycle: the filter-chain biquads switch coefficients at once, so a big jump
// clicks. 26/09: 4 steps 10 ms apart were shorter than the 21 ms quantum (1024
// frames at 48 kHz); half the steps landed in the same cycle and the gain moved
// in audible 0.5 dB+ jumps while a slider was dragged.
const RAMP_STEPS: u32 = 4;
const RAMP_MAX_STEP_DB: f32 = 0.25;
const RAMP_MAX_STEPS: u32 = 48;
const RAMP_STEP_INTERVAL: Duration = Duration::from_millis(22);

fn backend_schema() -> u8 {
    3
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
struct BackendState {
    #[serde(default = "backend_schema")]
    schema: u8,
    target_node_name: String,
    target_object_serial: Option<u64>,
    default_sink_before: Option<String>,
    routing_paused: bool,
    routed_streams: Vec<RoutedStream>,
    unproven_streams: Vec<StreamIdentity>,
    proven_stream_keys: Vec<String>,
    last_route_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
struct StreamIdentity {
    object_serial: u64,
    stream_key: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
struct RoutedStream {
    object_serial: u64,
    stream_key: String,
    original_sink_name: String,
    original_sink_serial: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct Target {
    id: u32,
    object_serial: u64,
    node_name: String,
}

#[derive(Debug, Clone, PartialEq)]
struct PulseSink {
    object_serial: u64,
    name: String,
}

#[derive(Debug, Clone, PartialEq)]
struct PulseInput {
    object_serial: u64,
    sink_serial: u64,
    stream_key: String,
    node_name: String,
    media_name: String,
}

/// An ordered, side-effect-free description of a routing update.  Keeping
/// persistence explicit is important: a route is recorded before its stream
/// is moved, so a supervisor restart cannot mistake its own move for a new
/// user-selected Game stream.
#[derive(Debug, Clone, PartialEq)]
enum RouteAction {
    SetDefaultSink(String),
    /// The state as it stood when this action was planned.  In particular,
    /// this must precede the corresponding stream move, rather than writing
    /// the final state of a batch of moves.
    PersistState(BackendState),
    MoveSinkInput {
        input_serial: u64,
        sink_name: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct RoutePlan {
    state: BackendState,
    actions: Vec<RouteAction>,
}

struct Health<'a> {
    service: bool,
    virtual_node: Option<&'a Target>,
    game: Option<&'a Target>,
    target_connected: bool,
    default_safe: bool,
    chat_isolated: bool,
    routing_healthy: bool,
    route_error: Option<&'a str>,
    applied: Option<&'a [Band]>,
    profile_synced: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub active: bool,
    pub service_active: bool,
    pub target_connected: bool,
    pub default_safe: bool,
    pub chat_isolated: bool,
    pub routing_healthy: bool,
    pub target_node_name: Option<String>,
    pub target_object_serial: Option<u64>,
    pub node_id: Option<u32>,
    pub object_serial: Option<u64>,
    pub virtual_sink_name: &'static str,
    pub routing: &'static str,
    pub applied_bands: Option<Vec<Band>>,
    pub profile_synced: bool,
    pub error: Option<String>,
}

/// Read-only health report for the optional spatial graph. Every false field
/// is intentional: absence, ambiguity, an unreadable graph, or an unsafe
/// route must never be promoted to an active renderer.
#[derive(Debug, Clone, Serialize)]
pub struct SpatialStatus {
    pub configured: bool,
    pub enabled: bool,
    pub active: bool,
    pub processing_mode: &'static str,
    pub service_active: bool,
    pub dataset_valid: bool,
    pub target_equalizer_connected: bool,
    pub default_safe: bool,
    pub chat_isolated: bool,
    pub capture_isolated: bool,
    pub routing_healthy: bool,
    pub graph_observable: bool,
    pub input_format_7_1: bool,
    pub output_format_stereo: bool,
    pub input_node_id: Option<u32>,
    pub input_object_serial: Option<u64>,
    pub output_node_id: Option<u32>,
    pub output_object_serial: Option<u64>,
    pub target_equalizer_node_id: Option<u32>,
    pub target_equalizer_object_serial: Option<u64>,
    pub target_equalizer_node_name: &'static str,
    pub error: Option<String>,
}

/// Inputs that collectively determine whether the spatial graph is safe to
/// advertise as active. Keeping these observations together makes it harder
/// to accidentally omit a safety gate when the health policy evolves.
struct SpatialHealth<'a> {
    service_active: bool,
    configured: bool,
    graph_observable: bool,
    input_format_7_1: bool,
    output_format_stereo: bool,
    target_equalizer_connected: bool,
    default_safe: bool,
    chat_isolated: bool,
    capture_isolated: bool,
    routing_healthy: bool,
    observation_error: Option<&'a str>,
}

fn path(name: &str) -> Result<PathBuf, String> {
    Ok(crate::equalizer::config_directory()?.join(name))
}

fn state_path() -> Result<PathBuf, String> {
    path(STATE_NAME)
}

fn config_path() -> Result<PathBuf, String> {
    path(CONFIG_NAME)
}

fn route_lock() -> Result<File, String> {
    #[cfg(test)]
    if MOCK_STATE.with(|state| state.borrow().is_some()) {
        // Unit tests keep routing state in-memory. Locking /dev/null preserves
        // the same FileExt synchronization contract without creating files in
        // the developer's configuration directory.
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .map_err(|error| format!("could not open test route lock: {error}"))?;
        lock.lock()
            .map_err(|error| format!("could not lock test route lock: {error}"))?;
        return Ok(lock);
    }
    let lock_path = path(LOCK_NAME)?;
    let directory = lock_path
        .parent()
        .ok_or("invalid PipeWire route lock path")?;
    fs::create_dir_all(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("{}: {error}", lock_path.display()))?;
    lock.lock()
        .map_err(|error| format!("could not lock {}: {error}", lock_path.display()))?;
    Ok(lock)
}

/// Every short-lived client runs with the bounded budget from
/// [`crate::command`]. The routing paths below call them while holding the
/// route lock, so an unbounded wait here would block every other equalizer and
/// spatial operation instead of failing with an actionable error.
fn command_output(program: &str, args: &[&str]) -> Result<Output, String> {
    crate::command::run_default(program, args)
}

fn command_failure(program: &str, output: &Output) -> String {
    crate::command::failure(program, output)
}

fn systemctl(args: &[&str]) -> Result<Output, String> {
    let mut all_args = vec!["--user"];
    all_args.extend_from_slice(args);
    command_output("systemctl", &all_args)
}

fn named_service_active(unit: &str) -> Result<bool, String> {
    let output = systemctl(&["is-active", "--quiet", unit])?;
    if output.status.success() {
        return Ok(true);
    }
    match output.status.code() {
        Some(3 | 4) => Ok(false),
        _ => Err(command_failure("systemctl --user is-active", &output)),
    }
}

fn service_active() -> Result<bool, String> {
    named_service_active(UNIT_NAME)
}

fn pw_dump() -> Result<Vec<Value>, String> {
    let output = command_output("pw-dump", &[])?;
    if !output.status.success() {
        return Err(command_failure("pw-dump", &output));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("pw-dump returned invalid JSON: {error}"))
}

fn pw_dump_node(node_id: u32) -> Result<Vec<Value>, String> {
    let id_str = node_id.to_string();
    let output = command_output("pw-dump", &[&id_str])?;
    if !output.status.success() {
        return Err(command_failure("pw-dump", &output));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("pw-dump returned invalid JSON: {error}"))
}

fn pactl_json(args: &[&str]) -> Result<Value, String> {
    let mut all_args = vec!["--format=json"];
    all_args.extend_from_slice(args);
    let output = command_output("pactl", &all_args)?;
    if !output.status.success() {
        return Err(command_failure("pactl", &output));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("pactl returned invalid JSON: {error}"))
}

fn pactl_default_sink() -> Result<String, String> {
    let output = command_output("pactl", &["get-default-sink"])?;
    if !output.status.success() {
        return Err(command_failure("pactl get-default-sink", &output));
    }
    let name = String::from_utf8(output.stdout)
        .map_err(|error| format!("pactl returned a non-UTF-8 default sink: {error}"))?
        .trim()
        .to_owned();
    if name.is_empty() {
        Err("pactl returned an empty default sink".into())
    } else {
        Ok(name)
    }
}

fn pactl_success(args: &[&str]) -> Result<(), String> {
    let output = command_output("pactl", args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failure("pactl", &output))
    }
}

#[cfg(test)]
std::thread_local! {
    static MOCK_STATE: std::cell::RefCell<Option<BackendState>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn install_mock_state_for_test() {
    MOCK_STATE.with(|state| *state.borrow_mut() = Some(BackendState::default()));
}

#[cfg(test)]
pub(crate) fn clear_mock_state_for_test() {
    MOCK_STATE.with(|state| *state.borrow_mut() = None);
}

fn read_state() -> Result<Option<BackendState>, String> {
    #[cfg(test)]
    {
        if let Some(state) = MOCK_STATE.with(|m| m.borrow().clone()) {
            return Ok(Some(state));
        }
    }
    let state = state_path()?;
    match fs::read_to_string(&state) {
        Ok(contents) => serde_json::from_str(&contents)
            .map(Some)
            .map_err(|error| format!("{}: {error}", state.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", state.display())),
    }
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let directory = path.parent().ok_or("invalid PipeWire state path")?;
    fs::create_dir_all(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&temporary, contents).map_err(|error| format!("{}: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| format!("{}: {error}", path.display()))
}

fn write_state(state: &BackendState) -> Result<(), String> {
    #[cfg(test)]
    {
        if MOCK_STATE.with(|m| m.borrow().is_some()) {
            MOCK_STATE.with(|m| *m.borrow_mut() = Some(state.clone()));
            return Ok(());
        }
    }
    let mut state = state.clone();
    state.schema = backend_schema();
    let serialized = serde_json::to_string_pretty(&state).map_err(|error| error.to_string())?;
    write_atomic(&state_path()?, &format!("{serialized}\n"))
}

fn pipewire_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn contains_word(value: &str, word: &str) -> bool {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|part| part.eq_ignore_ascii_case(word))
}

fn props(node: &Value) -> Option<&Map<String, Value>> {
    node.get("info")?.get("props")?.as_object()
}

fn property_i64(properties: &Map<String, Value>, name: &str) -> Option<i64> {
    properties.get(name).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str()?.parse::<i64>().ok())
    })
}

fn target_from_node(node: &Value) -> Option<Target> {
    let id = u32::try_from(node.get("id")?.as_u64()?).ok()?;
    let properties = props(node)?;
    Some(Target {
        id,
        object_serial: properties.get("object.serial")?.as_u64()?,
        node_name: properties.get("node.name")?.as_str()?.to_owned(),
    })
}

fn game_sink(node: &Value) -> Option<Target> {
    if node.get("type")?.as_str()? != "PipeWire:Interface:Node" {
        return None;
    }
    let properties = props(node)?;
    if properties.get("media.class")?.as_str()? != "Audio/Sink" {
        return None;
    }
    let node_name = properties.get("node.name")?.as_str()?;
    if node_name == VIRTUAL_SINK || contains_word(node_name, "chat") {
        return None;
    }
    let quantum_game_pcm = properties.get("alsa.components").and_then(Value::as_str)
        == Some(QUANTUM_ALSA_COMPONENTS)
        && property_i64(properties, "alsa.device") == Some(0)
        && properties
            .get("api.alsa.pcm.stream")
            .and_then(Value::as_str)
            == Some("playback");
    quantum_game_pcm.then(|| target_from_node(node)).flatten()
}

fn discover_game_sink(nodes: &[Value]) -> Result<Target, String> {
    let matches = nodes.iter().filter_map(game_sink).collect::<Vec<_>>();
    match matches.as_slice() {
        [target] => Ok(target.clone()),
        [] => Err("Quantum Game is unavailable; connect the JBL Quantum 810 and expose USB playback PCM 0, then the automatic equalizer service will retry".into()),
        _ => Err("multiple PipeWire Game sinks were found; refusing to choose a route automatically".into()),
    }
}

fn named_node(nodes: &[Value], name: &str) -> Option<Target> {
    nodes.iter().find_map(|node| {
        (props(node)?.get("node.name")?.as_str()? == name)
            .then(|| target_from_node(node))
            .flatten()
    })
}

fn quantum_non_game_node_ids(nodes: &[Value]) -> HashSet<u32> {
    nodes
        .iter()
        .filter_map(|node| {
            let properties = props(node)?;
            if properties.get("alsa.components").and_then(Value::as_str)
                != Some(QUANTUM_ALSA_COMPONENTS)
            {
                return None;
            }
            let is_game = properties.get("media.class").and_then(Value::as_str)
                == Some("Audio/Sink")
                && property_i64(properties, "alsa.device") == Some(0)
                && properties
                    .get("api.alsa.pcm.stream")
                    .and_then(Value::as_str)
                    == Some("playback");
            (!is_game).then(|| target_from_node(node).map(|target| target.id))?
        })
        .collect()
}

fn link_node_pair(link: &Value) -> Option<(u32, u32)> {
    if link.get("type")?.as_str()? != "PipeWire:Interface:Link" {
        return None;
    }
    let info = link.get("info")?;
    let output = u32::try_from(info.get("output-node-id")?.as_u64()?).ok()?;
    let input = u32::try_from(info.get("input-node-id")?.as_u64()?).ok()?;
    Some((output, input))
}

fn link_endpoints(link: &Value) -> Option<(u32, u32, bool)> {
    let (output, input) = link_node_pair(link)?;
    let info = link.get("info")?;
    let active = info.get("state").and_then(Value::as_str) == Some("active");
    Some((output, input, active))
}

fn nodes_named(nodes: &[Value], name: &str) -> Vec<Target> {
    nodes
        .iter()
        .filter_map(|node| {
            (props(node)?.get("node.name")?.as_str()? == name)
                .then(|| target_from_node(node))
                .flatten()
        })
        .collect()
}

fn single_named_node(nodes: &[Value], name: &str) -> Option<Target> {
    let matches = nodes_named(nodes, name);
    (matches.len() == 1)
        .then(|| matches.into_iter().next())
        .flatten()
}

fn single_named_audio_sink(nodes: &[Value], name: &str) -> Option<Target> {
    let matches = nodes_named(nodes, name)
        .into_iter()
        .filter(|target| {
            nodes.iter().any(|node| {
                node.get("id").and_then(Value::as_u64) == Some(u64::from(target.id))
                    && props(node)
                        .and_then(|properties| properties.get("media.class"))
                        .and_then(Value::as_str)
                        == Some("Audio/Sink")
            })
        })
        .collect::<Vec<_>>();
    (matches.len() == 1)
        .then(|| matches.into_iter().next())
        .flatten()
}

fn audio_channels(nodes: &[Value], node_id: u32) -> Option<i64> {
    nodes.iter().find_map(|node| {
        (node.get("id")?.as_u64() == Some(u64::from(node_id)))
            .then(|| property_i64(props(node)?, "audio.channels"))
            .flatten()
    })
}

fn node_is_chat_or_capture(node: &Value) -> (bool, bool) {
    let Some(properties) = props(node) else {
        return (false, false);
    };
    let name = properties
        .get("node.name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let media_class = properties
        .get("media.class")
        .and_then(Value::as_str)
        .unwrap_or_default();
    (
        contains_word(name, "chat"),
        media_class.contains("Source") || contains_word(name, "capture"),
    )
}

fn spatial_error(
    health: &SpatialHealth<'_>,
    capability: &crate::spatial::CapabilityReport,
) -> Option<String> {
    if !health.configured {
        return Some("spatial mode is not configured for binaural-stereo".into());
    }
    if !capability.dataset.valid {
        return Some(
            capability
                .dataset
                .error
                .clone()
                .unwrap_or_else(|| "the spatial HRTF dataset is invalid".into()),
        );
    }
    if !capability.ready {
        return capability.error.clone();
    }
    if !health.service_active {
        return Some(format!(
            "spatial lifecycle service is inactive: {SPATIAL_UNIT_NAME}"
        ));
    }
    if let Some(error) = health.observation_error {
        return Some(format!("spatial graph cannot be observed: {error}"));
    }
    if !health.graph_observable {
        return Some("the spatial input/output nodes are absent or ambiguous".into());
    }
    if !health.input_format_7_1 || !health.output_format_stereo {
        return Some(
            "the spatial graph does not expose 7.1 input and stereo output formats".into(),
        );
    }
    if !health.target_equalizer_connected {
        return Some("the spatial stereo output is not linked to both equalizer channels".into());
    }
    if !health.default_safe {
        return Some("the spatial virtual sink became the default sink".into());
    }
    if !health.chat_isolated || !health.capture_isolated {
        return Some("unsafe spatial route detected to Chat or a capture node".into());
    }
    if !health.routing_healthy {
        return Some("spatial routing contains an unexpected active link".into());
    }
    None
}

/// Reports what the observable filter controls are actually doing, rather
/// than echoing the persisted gate while a live update is pending or failed.
fn spatial_processing_mode(nodes: &[Value], input: Option<&Target>) -> &'static str {
    let Some(input) = input else { return "unknown" };
    let Some(node) = nodes
        .iter()
        .find(|node| node.get("id").and_then(Value::as_u64) == Some(u64::from(input.id)))
    else {
        return "unknown";
    };
    spatial_mix(node)
        .map(|mix| mix.processing_mode().as_str())
        .unwrap_or("unknown")
}

/// Inspects the spatial lifecycle and graph without changing PipeWire state.
/// Command failures become a fail-closed status report instead of hiding the
/// health fields from scripts and the widget.
pub fn spatial_status(
    profile: &crate::spatial::SpatialProfile,
    capability: &crate::spatial::CapabilityReport,
) -> SpatialStatus {
    let configured = profile.mode == crate::spatial::SpatialMode::BinauralStereo;
    let enabled = profile.enabled;
    let service = named_service_active(SPATIAL_UNIT_NAME).unwrap_or(false);
    let mut observation_error = None;
    let mut input = None;
    let mut output = None;
    let mut target = None;
    let mut input_format_7_1 = false;
    let mut output_format_stereo = false;
    let mut target_equalizer_connected = false;
    // With an observable PipeWire graph, absence of a spatial link is itself
    // isolation; graph_observable/routing_healthy separately state that the
    // required graph and route exist.
    let mut chat_isolated = true;
    let mut capture_isolated = true;
    let mut routing_healthy = false;
    let mut processing_mode = "unknown";

    match pw_dump() {
        Ok(nodes) => {
            input = single_named_node(&nodes, SPATIAL_SINK_NODE);
            output = single_named_node(&nodes, SPATIAL_OUTPUT_NODE);
            target = single_named_audio_sink(&nodes, TARGET_EQUALIZER_SINK);
            processing_mode = spatial_processing_mode(&nodes, input.as_ref());
            input_format_7_1 = input
                .as_ref()
                .and_then(|node| audio_channels(&nodes, node.id))
                == Some(8);
            output_format_stereo = output
                .as_ref()
                .and_then(|node| audio_channels(&nodes, node.id))
                == Some(2);

            if let Some(output_node) = output.as_ref() {
                // Passive filter-chain links are normally `paused` while no
                // stream is playing. Health describes the graph topology, so
                // every existing link must count here. This also makes the
                // safety guard detect an idle link to Chat or capture nodes.
                let graph_links = nodes.iter().filter_map(link_node_pair).collect::<Vec<_>>();
                let output_destinations = graph_links
                    .iter()
                    .filter_map(|(from, to)| (*from == output_node.id).then_some(*to))
                    .collect::<Vec<_>>();
                let expected_target = target.as_ref().map(|node| node.id);
                target_equalizer_connected = expected_target.is_some_and(|target_id| {
                    output_destinations
                        .iter()
                        .filter(|destination| **destination == target_id)
                        .count()
                        == 2
                });
                routing_healthy = target_equalizer_connected
                    && output_destinations.len() == 2
                    && expected_target.is_some_and(|target_id| {
                        output_destinations
                            .iter()
                            .all(|destination| *destination == target_id)
                    });

                let graph_ids = [input.as_ref().map(|node| node.id), Some(output_node.id)];
                let mut chat = false;
                let mut capture = false;
                for (from, to) in graph_links {
                    for (graph_id, other_id) in [(from, to), (to, from)] {
                        if graph_ids.contains(&Some(graph_id))
                            && let Some(other) = nodes.iter().find(|node| {
                                node.get("id").and_then(Value::as_u64) == Some(u64::from(other_id))
                            })
                        {
                            let (is_chat, is_capture) = node_is_chat_or_capture(other);
                            chat |= is_chat;
                            capture |= is_capture;
                        }
                    }
                }
                chat_isolated = !chat;
                capture_isolated = !capture;
            }
        }
        Err(error) => {
            observation_error = Some(error);
            chat_isolated = false;
            capture_isolated = false;
        }
    }

    let default_safe = match pactl_default_sink() {
        Ok(default) => default != SPATIAL_SINK_NODE && default != SPATIAL_OUTPUT_NODE,
        Err(error) => {
            observation_error.get_or_insert(error);
            false
        }
    };
    let graph_observable = input.is_some() && output.is_some() && target.is_some();
    let health = SpatialHealth {
        service_active: service,
        configured,
        graph_observable,
        input_format_7_1,
        output_format_stereo,
        target_equalizer_connected,
        default_safe,
        chat_isolated,
        capture_isolated,
        routing_healthy,
        observation_error: observation_error.as_deref(),
    };
    let error = spatial_error(&health, capability);
    let active = error.is_none();

    SpatialStatus {
        configured,
        enabled,
        active,
        processing_mode,
        service_active: service,
        dataset_valid: capability.dataset.valid,
        target_equalizer_connected,
        default_safe,
        chat_isolated,
        capture_isolated,
        routing_healthy,
        graph_observable,
        input_format_7_1,
        output_format_stereo,
        input_node_id: input.as_ref().map(|node| node.id),
        input_object_serial: input.as_ref().map(|node| node.object_serial),
        output_node_id: output.as_ref().map(|node| node.id),
        output_object_serial: output.as_ref().map(|node| node.object_serial),
        target_equalizer_node_id: target.as_ref().map(|node| node.id),
        target_equalizer_object_serial: target.as_ref().map(|node| node.object_serial),
        target_equalizer_node_name: TARGET_EQUALIZER_SINK,
        error,
    }
}

fn route_health(nodes: &[Value], output: Option<&Target>, game: Option<&Target>) -> (bool, bool) {
    let Some(output) = output else {
        return (false, true);
    };
    let non_game = quantum_non_game_node_ids(nodes);
    let links = nodes
        .iter()
        .filter_map(link_endpoints)
        .filter(|(_, _, active)| *active);
    let mut game_links = 0_u8;
    let mut isolated = true;
    for (output_id, input_id, _) in links {
        if output_id == output.id {
            if game.is_some_and(|target| input_id == target.id) {
                game_links = game_links.saturating_add(1);
            } else if non_game.contains(&input_id) {
                isolated = false;
            }
        }
    }
    (game_links >= 2, isolated)
}

fn applied_bands(node: &Value) -> Option<Vec<Band>> {
    let props_values = node.get("info")?.get("params")?.get("Props")?.as_array()?;
    let mut gains = HashMap::<u32, f32>::new();
    for object in props_values {
        let Some(params) = object.get("params").and_then(Value::as_array) else {
            continue;
        };
        let (pairs, _) = params.as_chunks::<2>();
        for pair in pairs {
            let Some(key) = pair[0].as_str() else {
                continue;
            };
            let Some(index) = key
                .strip_prefix("eq_band_")
                .and_then(|value| value.strip_suffix(":Gain"))
                .and_then(|value| value.parse::<usize>().ok())
            else {
                continue;
            };
            let Some(&frequency_hz) = BANDS_HZ.get(index) else {
                continue;
            };
            let Some(gain_db) = pair[1].as_f64() else {
                continue;
            };
            gains.insert(frequency_hz, gain_db as f32);
        }
    }
    BANDS_HZ
        .into_iter()
        .map(|frequency_hz| {
            gains.get(&frequency_hz).copied().map(|gain_db| Band {
                frequency_hz,
                gain_db,
            })
        })
        .collect()
}

fn graph_applied_bands(nodes: &[Value], node_id: u32) -> Option<Vec<Band>> {
    nodes
        .iter()
        .find(|node| node.get("id").and_then(Value::as_u64) == Some(u64::from(node_id)))
        .and_then(applied_bands)
}

fn profile_matches(applied: &[Band], expected: &EqualizerProfile) -> bool {
    applied.len() == expected.bands.len()
        && applied
            .iter()
            .zip(&expected.bands)
            .all(|(actual, expected)| {
                actual.frequency_hz == expected.frequency_hz
                    && (actual.gain_db - expected.gain_db).abs() <= 0.05
            })
}

fn gains_match(applied: &[Band], expected: &[f32]) -> bool {
    applied.len() == BANDS_HZ.len()
        && expected.len() == BANDS_HZ.len()
        && applied.iter().zip(BANDS_HZ.into_iter().zip(expected)).all(
            |(actual, (frequency_hz, gain_db))| {
                actual.frequency_hz == frequency_hz && (actual.gain_db - gain_db).abs() <= 0.05
            },
        )
}

fn render(profile: &EqualizerProfile, target: &str) -> String {
    let mut nodes = String::new();
    let mut links = String::new();
    for (index, band) in profile.bands.iter().enumerate() {
        let label = if index == 0 {
            "bq_lowshelf"
        } else if index + 1 == profile.bands.len() {
            "bq_highshelf"
        } else {
            "bq_peaking"
        };
        nodes.push_str(&format!(
            "                    {{ type = builtin name = eq_band_{index} label = {label} control = {{ \"Freq\" = {:.1} \"Q\" = 1.0 \"Gain\" = {:.1} }} }}\n",
            band.frequency_hz, band.gain_db
        ));
        if index > 0 {
            links.push_str(&format!(
                "                    {{ output = \"eq_band_{}:Out\" input = \"eq_band_{index}:In\" }}\n",
                index - 1
            ));
        }
    }
    let base = r#"context.properties = { log.level = 0 }
context.spa-libs = {
    audio.convert.* = audioconvert/libspa-audioconvert
    support.* = support/libspa-support
}
"#;
    let filter = format!(
        "# Generated by JamBaLinux SonicCore. Do not edit while the equalizer is active.\n# Q=1.0 and shelf endpoints are an initial approximation, not QuantumENGINE parity.\ncontext.modules = [\n    {{ name = libpipewire-module-filter-chain\n        args = {{\n            node.description = \"JamBaLinux Game Equalizer\"\n            media.name = \"JamBaLinux Game Equalizer\"\n            filter.graph = {{\n                nodes = [\n{nodes}                ]\n                links = [\n{links}                ]\n            }}\n            audio.channels = 2\n            audio.position = [ FL FR ]\n            capture.props = {{\n                node.name = \"{VIRTUAL_SINK}\"\n                node.description = \"JamBaLinux Game Equalizer\"\n                media.class = Audio/Sink\n                node.virtual = true\n                priority.session = 0\n                priority.driver = 0\n            }}\n            playback.props = {{\n                node.name = \"{OUTPUT_NODE}\"\n                node.passive = true\n                target.object = \"{}\"\n            }}\n        }}\n    }}\n]\n",
        pipewire_string(target)
    );
    let filter = filter.replacen(
        "context.modules = [\n",
        "context.modules = [\n    { name = libpipewire-module-rt flags = [ ifexists nofail ] }\n    { name = libpipewire-module-protocol-native }\n    { name = libpipewire-module-client-node }\n    { name = libpipewire-module-adapter }\n",
        1,
    );
    format!("{base}{filter}")
}

fn pulse_sinks(value: &Value) -> Result<Vec<PulseSink>, String> {
    let array = value
        .as_array()
        .ok_or("pactl list sinks did not return an array")?;
    array
        .iter()
        .map(|sink| {
            let object_serial = sink
                .get("index")
                .and_then(Value::as_u64)
                .ok_or("pactl sink is missing its index")?;
            let name = sink
                .get("name")
                .and_then(Value::as_str)
                .ok_or("pactl sink is missing its name")?
                .to_owned();
            Ok(PulseSink {
                object_serial,
                name,
            })
        })
        .collect()
}

fn pulse_inputs(value: &Value) -> Result<Vec<PulseInput>, String> {
    let array = value
        .as_array()
        .ok_or("pactl list sink-inputs did not return an array")?;
    array
        .iter()
        .map(|input| {
            let properties = input
                .get("properties")
                .and_then(Value::as_object)
                .ok_or("pactl sink input is missing properties")?;
            let object_serial = input
                .get("index")
                .and_then(Value::as_u64)
                .ok_or("pactl sink input is missing its index")?;
            let sink_serial = input
                .get("sink")
                .and_then(Value::as_u64)
                .ok_or("pactl sink input is missing its sink index")?;
            let property = |name: &str| {
                properties
                    .get(name)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            };
            let stream_key = [
                property("module-stream-restore.id"),
                property("application.process.id"),
                property("application.name"),
                property("node.name"),
            ]
            .join("|");
            Ok(PulseInput {
                object_serial,
                sink_serial,
                stream_key,
                node_name: property("node.name").to_owned(),
                media_name: property("media.name").to_owned(),
            })
        })
        .collect()
}

fn pulse_game_sink(value: &Value) -> Result<PulseSink, String> {
    let array = value
        .as_array()
        .ok_or("pactl list sinks did not return an array")?;
    let matches = array
        .iter()
        .filter_map(|sink| {
            let properties = sink.get("properties")?.as_object()?;
            let name = sink.get("name")?.as_str()?;
            if name == VIRTUAL_SINK || contains_word(name, "chat") {
                return None;
            }
            let exact = properties.get("alsa.components").and_then(Value::as_str)
                == Some(QUANTUM_ALSA_COMPONENTS)
                && property_i64(properties, "alsa.device") == Some(0)
                && properties
                    .get("api.alsa.pcm.stream")
                    .and_then(Value::as_str)
                    == Some("playback");
            if !exact {
                return None;
            }
            Some(PulseSink {
                object_serial: sink.get("index")?.as_u64()?,
                name: name.to_owned(),
            })
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [target] => Ok(target.clone()),
        [] => Err("Quantum Game is not present in the PulseAudio compatibility graph".into()),
        _ => Err("multiple Quantum Game sinks are present; refusing automatic routing".into()),
    }
}

fn is_equalizer_output(input: &PulseInput) -> bool {
    input.node_name == OUTPUT_NODE || input.media_name == "JamBaLinux Game Equalizer"
}

fn is_spatial_output(input: &PulseInput) -> bool {
    input.node_name == SPATIAL_OUTPUT_NODE
}

fn managed_route_destination(
    sink_serial: u64,
    equalizer_serial: u64,
    spatial_serial: Option<u64>,
) -> bool {
    sink_serial == equalizer_serial || spatial_serial == Some(sink_serial)
}

fn exact_spatial_sink(sinks: &[PulseSink]) -> Result<Option<&PulseSink>, String> {
    let matches = sinks
        .iter()
        .filter(|sink| sink.name == SPATIAL_SINK_NODE)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [sink] => Ok(Some(*sink)),
        _ => Err(format!(
            "multiple spatial sinks named {SPATIAL_SINK_NODE} exist; refusing automatic Game routing; remove the duplicate spatial sink"
        )),
    }
}

fn spatial_output_connected(inputs: &[PulseInput], equalizer_serial: u64) -> bool {
    let outputs = inputs
        .iter()
        .filter(|input| is_spatial_output(input))
        .collect::<Vec<_>>();
    matches!(outputs.as_slice(), [output] if output.sink_serial == equalizer_serial)
}

fn registered_route_needs_promotion(
    current_sink_serial: u64,
    equalizer_serial: u64,
    processing_sink_serial: u64,
) -> bool {
    current_sink_serial == equalizer_serial && processing_sink_serial != equalizer_serial
}

/// Records streams that the spatial supervisor is about to hand to the
/// equalizer. Only streams currently attached to the exact spatial sink are
/// accepted, and their eventual non-EQ fallback is the proven physical Game
/// endpoint. Registration happens before the move so the equalizer supervisor
/// cannot quarantine the stream during the handoff.
pub fn register_spatial_fallback_streams(input_serials: &[u64]) -> Result<(), String> {
    if input_serials.is_empty() {
        return Ok(());
    }
    let _route_lock = route_lock()?;
    let sinks_value = pactl_json(&["list", "sinks"])?;
    let sinks = pulse_sinks(&sinks_value)?;
    let spatial_matches = sinks
        .iter()
        .filter(|sink| sink.name == SPATIAL_SINK_NODE)
        .collect::<Vec<_>>();
    let spatial = match spatial_matches.as_slice() {
        [sink] => *sink,
        [] => return Err("the spatial sink disappeared before fallback registration".into()),
        _ => {
            return Err(
                "multiple spatial sinks exist; refusing to register fallback streams".into(),
            );
        }
    };
    let game = pulse_game_sink(&sinks_value)?;
    let inputs = pulse_inputs(&pactl_json(&["list", "sink-inputs"])?)?;
    let mut state = read_state()?.unwrap_or_default();

    for serial in input_serials {
        let input = inputs
            .iter()
            .find(|input| input.object_serial == *serial)
            .ok_or_else(|| format!("spatial stream {serial} disappeared before fallback"))?;
        if input.sink_serial != spatial.object_serial {
            return Err(format!(
                "stream {serial} is no longer attached to the exact spatial sink; refusing fallback registration"
            ));
        }
        state.unproven_streams.retain(|identity| {
            identity.object_serial != input.object_serial || identity.stream_key != input.stream_key
        });
        if !state.routed_streams.iter().any(|route| {
            route.object_serial == input.object_serial && route.stream_key == input.stream_key
        }) {
            if state.proven_stream_keys.contains(&input.stream_key) {
                state.routed_streams.push(RoutedStream {
                    object_serial: input.object_serial,
                    stream_key: input.stream_key.clone(),
                    original_sink_name: game.name.clone(),
                    original_sink_serial: game.object_serial,
                });
            }
        }
    }
    write_state(&state)
}

fn is_managed_processing_sink(name: &str) -> bool {
    name == VIRTUAL_SINK || name == SPATIAL_SINK_NODE
}

fn safe_default_sink<'a>(sinks: &'a [PulseSink], saved: Option<&str>) -> Option<&'a PulseSink> {
    saved
        .and_then(|name| {
            sinks
                .iter()
                .find(|sink| sink.name == name && !is_managed_processing_sink(&sink.name))
        })
        .or_else(|| {
            sinks
                .iter()
                .find(|sink| !is_managed_processing_sink(&sink.name))
        })
}

fn restore_target<'a>(sinks: &'a [PulseSink], route: &RoutedStream) -> Option<&'a PulseSink> {
    sinks
        .iter()
        .find(|sink| sink.name == route.original_sink_name)
        .or_else(|| {
            sinks
                .iter()
                .find(|sink| sink.object_serial == route.original_sink_serial)
        })
}

/// Plan shutdown restoration without overwriting a stream moved to a newer
/// user/session-manager destination. `routing_paused` is persisted by the
/// caller before the PipeWire snapshot is collected.
fn plan_restore_routes(
    sinks: &[PulseSink],
    inputs: &[PulseInput],
    mut state: BackendState,
) -> RoutePlan {
    let virtual_sink_serial = sinks
        .iter()
        .find(|sink| sink.name == VIRTUAL_SINK)
        .map(|sink| sink.object_serial);
    let mut actions = Vec::new();
    let mut remaining_routes = Vec::new();
    for route in &state.routed_streams {
        let Some(input) = inputs.iter().find(|input| {
            input.object_serial == route.object_serial && input.stream_key == route.stream_key
        }) else {
            // The stream ended; no restoration remains to perform.
            continue;
        };
        if Some(input.sink_serial) != virtual_sink_serial {
            // The user or session manager already moved it elsewhere. Respect
            // that newer destination instead of overwriting it on shutdown.
            continue;
        }
        if let Some(original) = restore_target(sinks, route) {
            actions.push(RouteAction::MoveSinkInput {
                input_serial: input.object_serial,
                sink_name: original.name.clone(),
            });
        } else {
            // Keep the proof and original destination for a later reconnect.
            remaining_routes.push(route.clone());
        }
    }
    state.routed_streams = remaining_routes;
    RoutePlan { state, actions }
}

/// Make the default safe before requiring the headset graph. This preserves
/// recovery when the Game endpoint is temporarily absent.
fn plan_default_recovery(
    sinks: &[PulseSink],
    default: &str,
    mut state: BackendState,
) -> Result<(RoutePlan, String), String> {
    if state.routing_paused {
        return Ok((
            RoutePlan {
                state,
                actions: Vec::new(),
            },
            default.to_owned(),
        ));
    }
    if is_managed_processing_sink(default) {
        let fallback = safe_default_sink(sinks, state.default_sink_before.as_deref()).ok_or(
            "a managed processing sink is the default and no physical fallback is available",
        )?;
        let effective_default = fallback.name.clone();
        return Ok((
            RoutePlan {
                state,
                actions: vec![RouteAction::SetDefaultSink(effective_default.clone())],
            },
            effective_default,
        ));
    }
    let changed = sinks
        .iter()
        .any(|sink| sink.name == default && !is_managed_processing_sink(&sink.name))
        && state.default_sink_before.as_deref() != Some(default);
    if changed {
        state.default_sink_before = Some(default.to_owned());
    }
    let actions = changed
        .then(|| RouteAction::PersistState(state.clone()))
        .into_iter()
        .collect();
    Ok((RoutePlan { state, actions }, default.to_owned()))
}

fn plan_route_iteration(
    sinks: &[PulseSink],
    inputs: &[PulseInput],
    game: &PulseSink,
    mut state: BackendState,
) -> Result<RoutePlan, String> {
    let mut actions = Vec::new();
    let mut changed = false;

    if state.routing_paused {
        return Ok(RoutePlan { state, actions });
    }

    // Default recovery must not depend on the headset being present. Only
    // after it is safe do we require the exact, confirmed Game sink.
    let virtual_sink = sinks
        .iter()
        .find(|sink| sink.name == VIRTUAL_SINK)
        .cloned()
        .ok_or("the automatic equalizer sink is not available yet")?;
    let spatial_sink = exact_spatial_sink(sinks)?;
    let spatial_sink_serial = spatial_sink.map(|sink| sink.object_serial);
    if state.target_node_name != game.name || state.target_object_serial != Some(game.object_serial)
    {
        state.target_node_name.clone_from(&game.name);
        state.target_object_serial = Some(game.object_serial);
        changed = true;
    }

    // The spatial capture sink can become visible just before its stereo
    // output is linked to the equalizer. Keep streams on the EQ during that
    // short startup window; the next iteration promotes registered routes as
    // soon as exactly one spatial output is connected to the exact EQ sink.
    let processing_sink = spatial_sink
        .filter(|_| spatial_output_connected(inputs, virtual_sink.object_serial))
        .unwrap_or(&virtual_sink);
    let old_len = state.routed_streams.len();
    state.routed_streams.retain(|route| {
        inputs.iter().any(|input| {
            input.object_serial == route.object_serial
                && input.stream_key == route.stream_key
                && managed_route_destination(
                    input.sink_serial,
                    virtual_sink.object_serial,
                    spatial_sink_serial,
                )
        })
    });
    changed |= state.routed_streams.len() != old_len;
    let old_unproven_len = state.unproven_streams.len();
    // A stream key can become proven in an earlier session while a stale
    // quarantine entry for the same live stream survives a service restart.
    // Keeping both makes the quarantine branch below win forever, so the
    // stream can sit on the physical Game sink without ever reaching the
    // equalizer or spatial graph. Historical proof is deliberately keyed by
    // application identity, so it is also sufficient to retire that stale
    // quarantine entry.
    let proven_stream_keys = state
        .proven_stream_keys
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    state.unproven_streams.retain(|identity| {
        !proven_stream_keys.contains(identity.stream_key.as_str())
            && inputs.iter().any(|input| {
                input.object_serial == identity.object_serial
                    && input.stream_key == identity.stream_key
            })
    });
    changed |= state.unproven_streams.len() != old_unproven_len;

    let evacuation_sink = safe_default_sink(sinks, state.default_sink_before.as_deref())
        .ok_or("no physical sink is available for an unproven equalizer stream")?;
    for input in inputs {
        let registered = state.routed_streams.iter().any(|route| {
            route.object_serial == input.object_serial && route.stream_key == input.stream_key
        });
        if input.sink_serial == virtual_sink.object_serial
            && !is_equalizer_output(input)
            && !is_spatial_output(input)
            && !registered
        {
            if state.proven_stream_keys.contains(&input.stream_key) {
                state.routed_streams.push(RoutedStream {
                    object_serial: input.object_serial,
                    stream_key: input.stream_key.clone(),
                    original_sink_name: game.name.clone(),
                    original_sink_serial: game.object_serial,
                });
                // Keep the pre-planner behavior: a stream restored by
                // WirePlumber is registered durably as soon as it is
                // recognized.  A later move in this iteration can fail.
                actions.push(RouteAction::PersistState(state.clone()));
                changed = true;
                continue;
            }

            if !state.unproven_streams.iter().any(|identity| {
                identity.object_serial == input.object_serial
                    && identity.stream_key == input.stream_key
            }) {
                state.unproven_streams.push(StreamIdentity {
                    object_serial: input.object_serial,
                    stream_key: input.stream_key.clone(),
                });
                // Persist the quarantine before moving the stream. A stream
                // evacuated to Game by this supervisor must not be mistaken
                // for proof that the user originally selected Game.
                actions.push(RouteAction::PersistState(state.clone()));
                changed = true;
            }
            // A stream is allowed through the virtual sink only after it was
            // observed on the exact Game endpoint and recorded below.
            actions.push(RouteAction::MoveSinkInput {
                input_serial: input.object_serial,
                sink_name: evacuation_sink.name.clone(),
            });
        }
    }

    for input in inputs {
        if is_equalizer_output(input) || is_spatial_output(input) {
            continue;
        }
        if state.unproven_streams.iter().any(|identity| {
            identity.object_serial == input.object_serial && identity.stream_key == input.stream_key
        }) {
            continue;
        }
        let registered = state.routed_streams.iter().any(|route| {
            route.object_serial == input.object_serial && route.stream_key == input.stream_key
        });
        if registered {
            if registered_route_needs_promotion(
                input.sink_serial,
                virtual_sink.object_serial,
                processing_sink.object_serial,
            ) {
                actions.push(RouteAction::MoveSinkInput {
                    input_serial: input.object_serial,
                    sink_name: processing_sink.name.clone(),
                });
                changed = true;
            }
            continue;
        }
        if input.sink_serial != game.object_serial {
            continue;
        }
        if !state.proven_stream_keys.contains(&input.stream_key) {
            state.proven_stream_keys.push(input.stream_key.clone());
            if state.proven_stream_keys.len() > 100 {
                state.proven_stream_keys.remove(0);
            }
        }
        state.routed_streams.push(RoutedStream {
            object_serial: input.object_serial,
            stream_key: input.stream_key.clone(),
            original_sink_name: game.name.clone(),
            original_sink_serial: game.object_serial,
        });
        // Persist the proven original destination before changing the route.
        actions.push(RouteAction::PersistState(state.clone()));
        actions.push(RouteAction::MoveSinkInput {
            input_serial: input.object_serial,
            sink_name: processing_sink.name.clone(),
        });
        changed = true;
    }
    if state.last_route_error.take().is_some() {
        changed = true;
    }
    if changed {
        actions.push(RouteAction::PersistState(state.clone()));
    }
    Ok(RoutePlan { state, actions })
}

fn execute_route_plan(plan: RoutePlan) -> Result<(), String> {
    for action in plan.actions {
        match action {
            RouteAction::SetDefaultSink(name) => pactl_success(&["set-default-sink", &name])?,
            RouteAction::PersistState(state) => write_state(&state)?,
            RouteAction::MoveSinkInput {
                input_serial,
                sink_name,
            } => pactl_success(&["move-sink-input", &input_serial.to_string(), &sink_name])?,
        }
    }
    Ok(())
}

fn run_route_iteration() -> Result<(), String> {
    let _route_lock = route_lock()?;
    let sinks_value = pactl_json(&["list", "sinks"])?;
    let sinks = pulse_sinks(&sinks_value)?;
    let default = pactl_default_sink()?;
    let state = read_state()?.unwrap_or_default();
    let (default_plan, _) = plan_default_recovery(&sinks, &default, state)?;
    let state = default_plan.state.clone();
    execute_route_plan(default_plan)?;
    if state.routing_paused {
        return Ok(());
    }
    let game = pulse_game_sink(&sinks_value)?;
    let inputs = pulse_inputs(&pactl_json(&["list", "sink-inputs"])?)?;
    execute_route_plan(plan_route_iteration(&sinks, &inputs, &game, state)?)
}

fn record_route_error(error: &str) -> Result<(), String> {
    let _route_lock = route_lock()?;
    let mut state = read_state()?.unwrap_or_default();
    if state.last_route_error.as_deref() != Some(error) {
        state.last_route_error = Some(error.to_owned());
        write_state(&state)?;
    }
    Ok(())
}

fn spawn_pipewire(config: &Path) -> Result<Child, String> {
    Command::new(crate::spatial::pipewire_binary()?)
        .arg("-c")
        .arg(config)
        .spawn()
        .map_err(|error| format!("could not start the persistent PipeWire equalizer: {error}"))
}

fn gain_payload(gains: &[f32]) -> String {
    let values = gains
        .iter()
        .enumerate()
        .map(|(index, gain)| format!("\"eq_band_{index}:Gain\" {gain:.3}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{{ params = [ {values} ] }}")
}

fn update_controls(node_id: u32, gains: &[f32]) -> Result<(), String> {
    let node = node_id.to_string();
    let payload = gain_payload(gains);
    let output = command_output("pw-cli", &["set-param", &node, "Props", &payload])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failure("pw-cli set-param", &output))
    }
}

fn profile_gains(profile: &EqualizerProfile) -> Vec<f32> {
    profile.bands.iter().map(|band| band.gain_db).collect()
}

/// Steps needed so no band moves more than RAMP_MAX_STEP_DB per audio cycle;
/// never fewer than RAMP_STEPS, capped so a full-scale reset stays about a second.
fn ramp_steps(from: &[f32], to: &[f32]) -> u32 {
    let largest = from
        .iter()
        .zip(to)
        .map(|(start, end)| (end - start).abs())
        .fold(0.0_f32, f32::max);
    ((largest / RAMP_MAX_STEP_DB).ceil() as u32).clamp(RAMP_STEPS, RAMP_MAX_STEPS)
}

fn ramp_controls(node_id: u32, from: &[f32], to: &[f32]) -> Result<(), String> {
    if from.len() != BANDS_HZ.len() || to.len() != BANDS_HZ.len() {
        return Err("the PipeWire ramp requires exactly ten bands".into());
    }
    let steps = ramp_steps(from, to);
    for step in 1..=steps {
        let alpha = step as f32 / steps as f32;
        let gains = from
            .iter()
            .zip(to)
            .map(|(start, end)| start + ((end - start) * alpha))
            .collect::<Vec<_>>();
        update_controls(node_id, &gains)?;
        if step < steps {
            thread::sleep(RAMP_STEP_INTERVAL);
        }
    }
    Ok(())
}

fn restore_controls(gains: &[f32]) -> Result<(), String> {
    let before = pw_dump()?;
    let node = named_node(&before, VIRTUAL_SINK)
        .ok_or("the equalizer node disappeared before rollback")?;
    update_controls(node.id, gains)?;
    let after = pw_dump()?;
    let same_node = named_node(&after, VIRTUAL_SINK)
        .filter(|after_node| {
            after_node.id == node.id && after_node.object_serial == node.object_serial
        })
        .ok_or("the equalizer node was recreated during rollback")?;
    let applied = graph_applied_bands(&after, same_node.id)
        .ok_or("the equalizer did not expose its controls after rollback")?;
    if gains_match(&applied, gains) {
        Ok(())
    } else {
        Err("the live controls do not match the rollback values".into())
    }
}

fn failed_update_with_rollback(error: impl Into<String>, previous: &[f32]) -> Result<(), String> {
    let error = error.into();
    match restore_controls(previous) {
        Ok(()) => Err(format!(
            "{error}; the preceding live DSP values were restored"
        )),
        Err(rollback) => Err(format!("{error}; DSP rollback also failed: {rollback}")),
    }
}

fn status_error(health: &Health<'_>) -> Option<String> {
    if !health.service {
        return Some(format!(
            "automatic equalizer service is inactive; run tools/install-user.sh or start {UNIT_NAME}"
        ));
    }
    if health.game.is_none() {
        return Some(
            "Quantum Game is unavailable; reconnect the JBL Quantum 810 and the service will retry automatically"
                .into(),
        );
    }
    if health.virtual_node.is_none() {
        return Some(format!(
            "the equalizer sink is missing; inspect systemctl --user status {UNIT_NAME}"
        ));
    }
    if !health.target_connected {
        return Some("the equalizer output is not linked to both Quantum Game channels".into());
    }
    if !health.chat_isolated {
        return Some(
            "unsafe PipeWire link detected: the equalizer reached Chat or a capture node".into(),
        );
    }
    if !health.default_safe {
        return Some(
            "the virtual equalizer became the default sink; automatic recovery is pending".into(),
        );
    }
    if let Some(error) = health.route_error {
        return Some(format!("automatic Game routing failed: {error}"));
    }
    if !health.routing_healthy {
        return Some(
            "automatic Game routing is pending or an unproven stream reached the equalizer".into(),
        );
    }
    if health.applied.is_none() {
        return Some("the equalizer DSP controls are not observable on the active sink".into());
    }
    if !health.profile_synced {
        return Some("the live DSP gains do not match the saved equalizer profile".into());
    }
    None
}

pub fn status(profile: &EqualizerProfile) -> Result<Status, String> {
    let service = service_active()?;
    let state = read_state()?;
    let nodes = pw_dump()?;
    let game = discover_game_sink(&nodes).ok();
    let virtual_node = named_node(&nodes, VIRTUAL_SINK);
    let output = named_node(&nodes, OUTPUT_NODE);
    let (graph_connected, graph_isolated) = route_health(&nodes, output.as_ref(), game.as_ref());
    let pulse_sinks_value = pactl_json(&["list", "sinks"])?;
    let pulse_sinks = pulse_sinks(&pulse_sinks_value)?;
    let pulse_game = pulse_game_sink(&pulse_sinks_value).ok();
    let pulse_virtual = pulse_sinks.iter().find(|sink| sink.name == VIRTUAL_SINK);
    let pulse_inputs = pulse_inputs(&pactl_json(&["list", "sink-inputs"])?)?;
    let equalizer_outputs = pulse_inputs
        .iter()
        .filter(|input| is_equalizer_output(input))
        .collect::<Vec<_>>();
    let pulse_connected = pulse_game.as_ref().is_some_and(|target| {
        equalizer_outputs
            .iter()
            .any(|input| input.sink_serial == target.object_serial)
    });
    let pulse_isolated = pulse_game.as_ref().is_some_and(|target| {
        !equalizer_outputs.is_empty()
            && equalizer_outputs
                .iter()
                .all(|input| input.sink_serial == target.object_serial)
    });
    let target_connected = graph_connected || pulse_connected;
    let chat_isolated = graph_isolated && pulse_isolated;
    let default_safe = !is_managed_processing_sink(&pactl_default_sink()?);
    let quarantined = |input: &PulseInput| {
        state.as_ref().is_some_and(|state| {
            state.unproven_streams.iter().any(|identity| {
                identity.object_serial == input.object_serial
                    && identity.stream_key == input.stream_key
            })
        })
    };
    let registered = |input: &PulseInput| {
        state.as_ref().is_some_and(|state| {
            state.routed_streams.iter().any(|route| {
                route.object_serial == input.object_serial && route.stream_key == input.stream_key
            })
        })
    };
    let game_pending = pulse_game.as_ref().is_some_and(|target| {
        pulse_inputs.iter().any(|input| {
            input.sink_serial == target.object_serial
                && !is_equalizer_output(input)
                && !quarantined(input)
        })
    });
    let virtual_unproven = pulse_virtual.is_some_and(|target| {
        pulse_inputs.iter().any(|input| {
            input.sink_serial == target.object_serial
                && !is_equalizer_output(input)
                && !is_spatial_output(input)
                && !registered(input)
        })
    });
    let route_error = state
        .as_ref()
        .and_then(|state| state.last_route_error.as_deref());
    let routing_healthy = !game_pending && !virtual_unproven && route_error.is_none();
    let applied = virtual_node
        .as_ref()
        .and_then(|node| graph_applied_bands(&nodes, node.id));
    let profile_synced = applied
        .as_deref()
        .is_some_and(|bands| profile_matches(bands, profile));
    let error = status_error(&Health {
        service,
        virtual_node: virtual_node.as_ref(),
        game: game.as_ref(),
        target_connected,
        default_safe,
        chat_isolated,
        routing_healthy,
        route_error,
        applied: applied.as_deref(),
        profile_synced,
    });
    let active = error.is_none();
    Ok(Status {
        active,
        service_active: service,
        target_connected,
        default_safe,
        chat_isolated,
        routing_healthy,
        target_node_name: game
            .as_ref()
            .map(|target| target.node_name.clone())
            .or_else(|| state.as_ref().map(|state| state.target_node_name.clone()))
            .filter(|name| !name.is_empty()),
        target_object_serial: game
            .as_ref()
            .map(|target| target.object_serial)
            .or_else(|| state.as_ref().and_then(|state| state.target_object_serial)),
        node_id: virtual_node.as_ref().map(|node| node.id),
        object_serial: virtual_node.as_ref().map(|node| node.object_serial),
        virtual_sink_name: VIRTUAL_SINK,
        routing: "automatic-proven-game-streams",
        applied_bands: applied,
        profile_synced,
        error,
    })
}

pub fn prepare(profile: &EqualizerProfile) -> Result<(), String> {
    let _route_lock = route_lock()?;
    let sinks_value = pactl_json(&["list", "sinks"])?;
    let sinks = pulse_sinks(&sinks_value)?;
    let default = pactl_default_sink()?;
    let mut state = read_state()?.unwrap_or_default();
    if is_managed_processing_sink(&default) {
        let fallback = safe_default_sink(&sinks, state.default_sink_before.as_deref()).ok_or(
            "a managed processing sink is the default and no physical fallback is available",
        )?;
        pactl_success(&["set-default-sink", &fallback.name])?;
    } else if sinks
        .iter()
        .any(|sink| sink.name == default && !is_managed_processing_sink(&sink.name))
    {
        state.default_sink_before = Some(default);
    }
    // Persist default recovery even while Game is disconnected and discovery
    // below remains in its retry loop.
    write_state(&state)?;

    let nodes = pw_dump()?;
    let target = discover_game_sink(&nodes)?;
    write_atomic(&config_path()?, &render(profile, &target.node_name))?;
    state.routing_paused = false;
    state.target_node_name.clone_from(&target.node_name);
    state.target_object_serial = Some(target.object_serial);
    write_state(&state)
}

pub fn run(profile: &EqualizerProfile) -> Result<(), String> {
    loop {
        match prepare(profile) {
            Ok(()) => break,
            Err(error) => {
                eprintln!("{error}");
                thread::sleep(RETRY_INTERVAL);
            }
        }
    }
    let config = config_path()?;
    let mut child = spawn_pipewire(&config)?;
    let mut last_route_error = String::new();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("could not monitor the PipeWire equalizer: {error}"))?
        {
            return Err(format!(
                "persistent PipeWire equalizer exited with {status}"
            ));
        }
        match run_route_iteration() {
            Ok(()) => last_route_error.clear(),
            Err(error) if error != last_route_error => {
                eprintln!("automatic Game routing: {error}");
                if let Err(state_error) = record_route_error(&error) {
                    eprintln!("could not persist routing error: {state_error}");
                }
                last_route_error = error;
            }
            Err(error) => {
                let _ = record_route_error(&error);
            }
        }
        thread::sleep(ROUTE_INTERVAL);
    }
}

pub fn apply_profile(
    profile: &EqualizerProfile,
    fallback_previous: &EqualizerProfile,
) -> Result<(), String> {
    let before = pw_dump()?;
    let node = named_node(&before, VIRTUAL_SINK).ok_or(format!(
        "the automatic equalizer sink is unavailable; inspect systemctl --user status {UNIT_NAME}"
    ))?;
    let current = graph_applied_bands(&before, node.id)
        .map(|bands| bands.into_iter().map(|band| band.gain_db).collect())
        .unwrap_or_else(|| profile_gains(fallback_previous));
    let desired = profile_gains(profile);
    if let Err(error) = ramp_controls(node.id, &current, &desired) {
        return failed_update_with_rollback(
            format!("could not update the live equalizer: {error}"),
            &current,
        );
    }

    let after = match pw_dump_node(node.id) {
        Ok(after) => after,
        Err(error) => {
            return failed_update_with_rollback(
                format!("could not verify the live equalizer: {error}"),
                &current,
            );
        }
    };
    let same_node = named_node(&after, VIRTUAL_SINK).filter(|after_node| {
        after_node.id == node.id && after_node.object_serial == node.object_serial
    });
    let Some(same_node) = same_node else {
        return failed_update_with_rollback(
            "the equalizer node was recreated during a live update; refusing to save the profile",
            &current,
        );
    };
    let Some(applied) = graph_applied_bands(&after, same_node.id) else {
        return failed_update_with_rollback(
            "the live DSP did not expose its applied gains after the update",
            &current,
        );
    };
    if !profile_matches(&applied, profile) {
        return failed_update_with_rollback(
            "the live DSP gains did not match the requested profile; the saved profile was not changed",
            &current,
        );
    }
    Ok(())
}

pub fn restore_routes() -> Result<(), String> {
    let _route_lock = route_lock()?;
    let Some(mut state) = read_state()? else {
        return Ok(());
    };
    // Stop the still-running supervisor from undoing restoration while
    // systemd executes ExecStop, before it terminates the service cgroup.
    state.routing_paused = true;
    write_state(&state)?;
    let sinks = pulse_sinks(&pactl_json(&["list", "sinks"])?)?;
    let inputs = pulse_inputs(&pactl_json(&["list", "sink-inputs"])?)?;
    let plan = plan_restore_routes(&sinks, &inputs, state);
    let state = plan.state.clone();
    execute_route_plan(plan)?;
    let default = pactl_default_sink()?;
    if is_managed_processing_sink(&default)
        && let Some(fallback) = safe_default_sink(&sinks, state.default_sink_before.as_deref())
    {
        pactl_success(&["set-default-sink", &fallback.name])?;
    }
    write_state(&state)
}

pub fn enable(profile: &EqualizerProfile) -> Result<Status, String> {
    prepare(profile)?;
    let reload = systemctl(&["daemon-reload"])?;
    if !reload.status.success() {
        return Err(command_failure("systemctl --user daemon-reload", &reload));
    }
    let start = systemctl(&["enable", "--now", UNIT_NAME])?;
    if !start.status.success() {
        return Err(format!(
            "could not start {UNIT_NAME}: {}",
            command_failure("systemctl --user enable", &start)
        ));
    }
    status(profile)
}

pub fn disable() -> Result<Status, String> {
    restore_routes()?;
    let stop = systemctl(&["disable", "--now", UNIT_NAME])?;
    if !stop.status.success() {
        return Err(command_failure("systemctl --user disable", &stop));
    }
    status(&crate::equalizer::load()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::graph::{SPATIAL_DRY_CONTROLS, SPATIAL_WET_CONTROLS};
    use std::sync::Arc;

    fn node(id: u32, serial: u64, properties: Value) -> Value {
        let mut properties = properties;
        properties["object.serial"] = Value::from(serial);
        serde_json::json!({
            "id": id,
            "type": "PipeWire:Interface:Node",
            "info": { "props": properties }
        })
    }

    #[test]
    fn graph_has_ten_approximate_biquads_and_protects_default_priority() {
        let graph = render(&EqualizerProfile::default(), "alsa_output.usb_Quantum_Game");
        assert_eq!(graph.matches("label = bq_").count(), 10);
        assert!(graph.contains("label = bq_lowshelf"));
        assert!(graph.contains("label = bq_highshelf"));
        assert!(graph.contains("target.object = \"alsa_output.usb_Quantum_Game\""));
        assert!(graph.contains("node.name = \"jambalinux-soniccore-game-equalizer\""));
        assert!(graph.contains("priority.session = 0"));
        assert_eq!(graph.matches("context.modules = [").count(), 1);
        assert!(graph.contains("libpipewire-module-protocol-native"));
    }

    #[test]
    fn a_game_label_without_the_confirmed_usb_pcm_is_rejected() {
        let labelled_game = node(
            10,
            100,
            serde_json::json!({
                "media.class": "Audio/Sink",
                "node.name": "alsa_output.usb_quantum_game",
                "node.description": "Quantum Game"
            }),
        );
        assert_eq!(game_sink(&labelled_game), None);
        let chat = node(
            11,
            101,
            serde_json::json!({
                "media.class": "Audio/Sink",
                "node.name": "alsa_output.usb_quantum_chat",
                "node.description": "Quantum Game Chat"
            }),
        );
        assert_eq!(game_sink(&chat), None);
    }

    #[test]
    fn safe_default_never_selects_a_managed_processing_sink() {
        let sinks = vec![
            PulseSink {
                object_serial: 1,
                name: VIRTUAL_SINK.into(),
            },
            PulseSink {
                object_serial: 4,
                name: SPATIAL_SINK_NODE.into(),
            },
            PulseSink {
                object_serial: 2,
                name: "physical-a".into(),
            },
            PulseSink {
                object_serial: 3,
                name: "physical-b".into(),
            },
        ];
        assert_eq!(
            safe_default_sink(&sinks, Some("physical-b")).map(|sink| sink.name.as_str()),
            Some("physical-b")
        );
        assert_eq!(
            safe_default_sink(&sinks, Some(VIRTUAL_SINK)).map(|sink| sink.name.as_str()),
            Some("physical-a")
        );
        assert_eq!(
            safe_default_sink(&sinks, Some(SPATIAL_SINK_NODE)).map(|sink| sink.name.as_str()),
            Some("physical-a")
        );
    }

    fn routing_sinks() -> (Vec<PulseSink>, PulseSink) {
        let game = PulseSink {
            object_serial: 200,
            name: "alsa_output.usb_quantum.game".into(),
        };
        (
            vec![
                game.clone(),
                PulseSink {
                    object_serial: 100,
                    name: VIRTUAL_SINK.into(),
                },
                PulseSink {
                    object_serial: 300,
                    name: "alsa_output.analog.safe".into(),
                },
            ],
            game,
        )
    }

    fn playback_input(serial: u64, sink_serial: u64, key: &str) -> PulseInput {
        PulseInput {
            object_serial: serial,
            sink_serial,
            stream_key: key.into(),
            node_name: "application.output".into(),
            media_name: "Application audio".into(),
        }
    }

    #[test]
    fn route_plan_records_a_new_game_stream_before_moving_it() {
        let (sinks, game) = routing_sinks();
        let plan = plan_route_iteration(
            &sinks,
            &[playback_input(10, game.object_serial, "game-stream")],
            &game,
            BackendState::default(),
        )
        .unwrap();

        assert_eq!(plan.state.routed_streams.len(), 1);
        assert_eq!(plan.state.proven_stream_keys, ["game-stream"]);
        assert!(matches!(plan.actions[0], RouteAction::PersistState(_)));
        assert!(matches!(
            plan.actions[1],
            RouteAction::MoveSinkInput { input_serial: 10, ref sink_name } if sink_name == VIRTUAL_SINK
        ));
    }

    #[test]
    fn route_plan_persists_each_route_snapshot_before_its_move() {
        let (sinks, game) = routing_sinks();
        let plan = plan_route_iteration(
            &sinks,
            &[
                playback_input(10, game.object_serial, "first"),
                playback_input(11, game.object_serial, "second"),
            ],
            &game,
            BackendState::default(),
        )
        .unwrap();

        assert!(matches!(
            &plan.actions[0],
            RouteAction::PersistState(state)
                if state.routed_streams.iter().map(|route| route.stream_key.as_str()).eq(["first"])
                    && state.last_route_error.is_none()
        ));
        assert!(matches!(
            &plan.actions[1],
            RouteAction::MoveSinkInput {
                input_serial: 10,
                ..
            }
        ));
        assert!(matches!(
            &plan.actions[2],
            RouteAction::PersistState(state)
                if state.routed_streams.iter().map(|route| route.stream_key.as_str()).eq(["first", "second"])
                    && state.last_route_error.is_none()
        ));
        assert!(matches!(
            &plan.actions[3],
            RouteAction::MoveSinkInput {
                input_serial: 11,
                ..
            }
        ));
    }

    #[test]
    fn route_plan_quarantines_an_unproven_virtual_stream_before_evacuation() {
        let (sinks, game) = routing_sinks();
        let plan = plan_route_iteration(
            &sinks,
            &[playback_input(11, 100, "unknown-stream")],
            &game,
            BackendState::default(),
        )
        .unwrap();

        assert_eq!(plan.state.unproven_streams.len(), 1);
        assert!(matches!(
            &plan.actions[0],
            RouteAction::PersistState(state)
                if state.unproven_streams.iter().map(|stream| stream.stream_key.as_str()).eq(["unknown-stream"])
        ));
        assert!(matches!(
            plan.actions[1],
            RouteAction::MoveSinkInput { input_serial: 11, ref sink_name } if sink_name == "alsa_output.usb_quantum.game"
        ));
    }

    #[test]
    fn route_plan_reconciles_stale_quarantine_for_a_proven_game_stream() {
        let (sinks, game) = routing_sinks();
        let state = BackendState {
            unproven_streams: vec![StreamIdentity {
                object_serial: 11,
                stream_key: "known-stream".into(),
            }],
            proven_stream_keys: vec!["known-stream".into()],
            ..BackendState::default()
        };
        let plan = plan_route_iteration(
            &sinks,
            &[playback_input(11, game.object_serial, "known-stream")],
            &game,
            state,
        )
        .unwrap();

        assert!(plan.state.unproven_streams.is_empty());
        assert_eq!(plan.state.routed_streams.len(), 1);
        assert!(matches!(
            plan.actions.get(1),
            Some(RouteAction::MoveSinkInput { input_serial: 11, sink_name })
                if sink_name == VIRTUAL_SINK
        ));
    }

    #[test]
    fn route_plan_promotes_registered_stream_when_spatial_is_ready() {
        let (mut sinks, game) = routing_sinks();
        sinks.push(PulseSink {
            object_serial: 400,
            name: SPATIAL_SINK_NODE.into(),
        });
        let mut spatial_output = playback_input(99, 100, "spatial-output");
        spatial_output.node_name = SPATIAL_OUTPUT_NODE.into();
        let state = BackendState {
            routed_streams: vec![RoutedStream {
                object_serial: 12,
                stream_key: "registered".into(),
                original_sink_name: game.name.clone(),
                original_sink_serial: game.object_serial,
            }],
            ..BackendState::default()
        };
        let plan = plan_route_iteration(
            &sinks,
            &[spatial_output, playback_input(12, 100, "registered")],
            &game,
            state,
        )
        .unwrap();

        assert!(matches!(
            plan.actions.as_slice(),
            [RouteAction::MoveSinkInput { input_serial: 12, sink_name } , RouteAction::PersistState(_)]
                if sink_name == SPATIAL_SINK_NODE
        ));
    }

    #[test]
    fn default_recovery_never_selects_a_managed_sink() {
        let sinks = vec![
            PulseSink {
                object_serial: 100,
                name: VIRTUAL_SINK.into(),
            },
            PulseSink {
                object_serial: 101,
                name: SPATIAL_SINK_NODE.into(),
            },
            PulseSink {
                object_serial: 200,
                name: "physical-safe".into(),
            },
        ];
        for recorded_default in [VIRTUAL_SINK, SPATIAL_SINK_NODE] {
            let state = BackendState {
                default_sink_before: Some(recorded_default.into()),
                ..BackendState::default()
            };
            let (plan, effective_default) =
                plan_default_recovery(&sinks, VIRTUAL_SINK, state).unwrap();

            assert!(matches!(
                plan.actions.first(),
                Some(RouteAction::SetDefaultSink(name)) if name == "physical-safe"
            ));
            assert_eq!(effective_default, "physical-safe");
            assert!(!plan.actions.iter().any(|action| matches!(
                action,
                RouteAction::SetDefaultSink(name) if is_managed_processing_sink(name)
            )));
        }
    }

    #[test]
    fn default_recovery_records_a_new_physical_default() {
        let (sinks, _) = routing_sinks();
        let (plan, effective_default) =
            plan_default_recovery(&sinks, "alsa_output.analog.safe", BackendState::default())
                .unwrap();

        assert_eq!(effective_default, "alsa_output.analog.safe");
        assert!(matches!(
            plan.actions.as_slice(),
            [RouteAction::PersistState(state)]
                if state.default_sink_before.as_deref() == Some("alsa_output.analog.safe")
        ));
    }

    #[test]
    fn default_recovery_fails_without_a_physical_fallback() {
        let sinks = vec![PulseSink {
            object_serial: 100,
            name: VIRTUAL_SINK.into(),
        }];
        let error =
            plan_default_recovery(&sinks, VIRTUAL_SINK, BackendState::default()).unwrap_err();
        assert!(error.contains("no physical fallback"));
    }

    #[test]
    fn default_recovery_does_nothing_while_routing_is_paused() {
        let (sinks, _) = routing_sinks();
        let state = BackendState {
            routing_paused: true,
            ..BackendState::default()
        };
        let (plan, effective_default) = plan_default_recovery(&sinks, VIRTUAL_SINK, state).unwrap();
        assert!(plan.actions.is_empty());
        assert_eq!(effective_default, VIRTUAL_SINK);
    }

    #[test]
    fn route_plan_recognizes_a_proven_stream_restored_to_the_equalizer() {
        let (sinks, game) = routing_sinks();
        let state = BackendState {
            proven_stream_keys: vec!["known".into()],
            ..BackendState::default()
        };
        let plan = plan_route_iteration(&sinks, &[playback_input(13, 100, "known")], &game, state)
            .unwrap();

        assert!(plan.state.unproven_streams.is_empty());
        assert_eq!(plan.state.routed_streams[0].object_serial, 13);
        assert!(matches!(
            plan.actions.first(),
            Some(RouteAction::PersistState(state))
                if state.routed_streams.iter().map(|route| route.stream_key.as_str()).eq(["known"])
        ));
        assert!(
            plan.actions
                .iter()
                .all(|action| !matches!(action, RouteAction::MoveSinkInput { .. }))
        );
    }

    #[test]
    fn restore_routes_prefers_the_recorded_sink_name_then_serial() {
        let route = RoutedStream {
            object_serial: 14,
            stream_key: "restore".into(),
            original_sink_name: "renamed-sink".into(),
            original_sink_serial: 300,
        };
        let sinks = vec![
            PulseSink {
                object_serial: 300,
                name: "fallback-by-serial".into(),
            },
            PulseSink {
                object_serial: 301,
                name: "renamed-sink".into(),
            },
        ];
        assert_eq!(
            restore_target(&sinks, &route).map(|sink| sink.name.as_str()),
            Some("renamed-sink")
        );

        let without_name = &sinks[..1];
        assert_eq!(
            restore_target(without_name, &route).map(|sink| sink.name.as_str()),
            Some("fallback-by-serial")
        );
    }

    #[test]
    fn restore_plan_preserves_a_stream_moved_to_a_newer_destination() {
        let (sinks, game) = routing_sinks();
        let state = BackendState {
            routed_streams: vec![RoutedStream {
                object_serial: 14,
                stream_key: "restore".into(),
                original_sink_name: game.name.clone(),
                original_sink_serial: game.object_serial,
            }],
            ..BackendState::default()
        };
        let plan = plan_restore_routes(&sinks, &[playback_input(14, 300, "restore")], state);

        assert!(
            plan.actions
                .iter()
                .all(|action| !matches!(action, RouteAction::MoveSinkInput { .. }))
        );
        assert!(plan.state.routed_streams.is_empty());
    }

    #[test]
    fn restore_plan_moves_a_virtual_stream_back_to_its_recorded_sink_name() {
        let (sinks, game) = routing_sinks();
        let state = BackendState {
            routed_streams: vec![RoutedStream {
                object_serial: 14,
                stream_key: "restore".into(),
                original_sink_name: game.name.clone(),
                original_sink_serial: game.object_serial,
            }],
            ..BackendState::default()
        };
        let plan = plan_restore_routes(&sinks, &[playback_input(14, 100, "restore")], state);

        assert!(matches!(
            plan.actions.as_slice(),
            [RouteAction::MoveSinkInput { input_serial: 14, sink_name }]
                if sink_name == &game.name
        ));
        assert!(plan.state.routed_streams.is_empty());
    }

    #[test]
    fn restore_plan_keeps_route_when_its_recorded_sink_is_missing() {
        let (mut sinks, game) = routing_sinks();
        sinks.retain(|sink| sink.object_serial != game.object_serial);
        let state = BackendState {
            routed_streams: vec![RoutedStream {
                object_serial: 14,
                stream_key: "restore".into(),
                original_sink_name: game.name,
                original_sink_serial: game.object_serial,
            }],
            ..BackendState::default()
        };
        let plan = plan_restore_routes(&sinks, &[playback_input(14, 100, "restore")], state);

        assert!(plan.actions.is_empty());
        assert_eq!(plan.state.routed_streams.len(), 1);
        assert_eq!(plan.state.routed_streams[0].stream_key, "restore");
    }

    #[test]
    fn spatial_route_requires_exactly_one_spatial_sink() {
        let none = vec![PulseSink {
            object_serial: 1,
            name: VIRTUAL_SINK.into(),
        }];
        assert_eq!(exact_spatial_sink(&none).unwrap(), None);

        let one = vec![PulseSink {
            object_serial: 2,
            name: SPATIAL_SINK_NODE.into(),
        }];
        assert_eq!(
            exact_spatial_sink(&one)
                .unwrap()
                .map(|sink| sink.object_serial),
            Some(2)
        );

        let duplicates = vec![
            PulseSink {
                object_serial: 2,
                name: SPATIAL_SINK_NODE.into(),
            },
            PulseSink {
                object_serial: 3,
                name: SPATIAL_SINK_NODE.into(),
            },
        ];
        let error = exact_spatial_sink(&duplicates).unwrap_err();
        assert!(error.contains("multiple spatial sinks"));
        assert!(error.contains("remove the duplicate"));
    }

    #[test]
    fn spatial_route_waits_for_one_output_connected_to_the_equalizer() {
        let output = |serial, sink_serial| PulseInput {
            object_serial: serial,
            sink_serial,
            stream_key: format!("spatial-{serial}"),
            node_name: SPATIAL_OUTPUT_NODE.into(),
            media_name: "JamBaLinux Game Spatial".into(),
        };

        assert!(!spatial_output_connected(&[], 100));
        assert!(!spatial_output_connected(&[output(1, 200)], 100));
        assert!(spatial_output_connected(&[output(1, 100)], 100));
        assert!(!spatial_output_connected(
            &[output(1, 100), output(2, 100)],
            100
        ));
    }

    #[test]
    fn a_registered_equalizer_route_is_promoted_when_spatial_becomes_ready() {
        assert!(registered_route_needs_promotion(100, 100, 200));
        assert!(!registered_route_needs_promotion(200, 100, 200));
        assert!(!registered_route_needs_promotion(100, 100, 100));
        assert!(!registered_route_needs_promotion(300, 100, 200));
    }

    #[test]
    fn quantum_usb_playback_pcm_zero_is_game_and_pcm_one_is_not() {
        let game = node(
            93,
            1820,
            serde_json::json!({
                "media.class": "Audio/Sink",
                "node.name": "alsa_output.usb_quantum.pro-output-0",
                "alsa.components": "USB0ecb:2069",
                "alsa.device": 0,
                "api.alsa.pcm.stream": "playback"
            }),
        );
        assert_eq!(game_sink(&game).unwrap().object_serial, 1820);
        let mut chat = game.clone();
        chat["info"]["props"]["alsa.device"] = Value::from(1);
        assert_eq!(game_sink(&chat), None);
    }

    #[test]
    fn applied_gains_are_read_from_the_dsp_props() {
        let mut params = Vec::new();
        for (index, frequency) in BANDS_HZ.into_iter().enumerate() {
            params.push(Value::from(format!("eq_band_{index}:Freq")));
            params.push(Value::from(frequency));
            params.push(Value::from(format!("eq_band_{index}:Gain")));
            params.push(Value::from(index as f64));
        }
        let value = serde_json::json!({
            "info": { "params": { "Props": [ { "params": params } ] } }
        });
        let bands = applied_bands(&value).unwrap();
        assert_eq!(bands.len(), 10);
        assert_eq!(bands[4].frequency_hz, 500);
        assert_eq!(bands[4].gain_db, 4.0);
    }

    #[test]
    fn one_control_payload_contains_all_ten_bands() {
        let payload = gain_payload(&[0.0; 10]);
        assert_eq!(payload.matches(":Gain").count(), 10);
        assert!(payload.starts_with("{ params = ["));
        assert!(payload.contains("\"eq_band_9:Gain\" 0.000"));
    }

    #[test]
    fn equalizer_output_is_never_selected_for_automatic_routing() {
        let input = PulseInput {
            object_serial: 42,
            sink_serial: 100,
            stream_key: "key".into(),
            node_name: OUTPUT_NODE.into(),
            media_name: "JamBaLinux Game Equalizer".into(),
        };
        assert!(is_equalizer_output(&input));
    }

    #[test]
    fn only_the_exact_spatial_output_is_authorized_at_the_equalizer_input() {
        let mut input = PulseInput {
            object_serial: 43,
            sink_serial: 100,
            stream_key: "key".into(),
            node_name: SPATIAL_OUTPUT_NODE.into(),
            media_name: "JamBaLinux Game Spatial".into(),
        };
        assert!(is_spatial_output(&input));

        input.node_name = "third-party-spatial-output".into();
        assert!(!is_spatial_output(&input));
    }

    #[test]
    fn managed_routes_survive_only_at_the_equalizer_or_exact_spatial_sink() {
        assert!(managed_route_destination(100, 100, Some(200)));
        assert!(managed_route_destination(200, 100, Some(200)));
        assert!(!managed_route_destination(300, 100, Some(200)));
        assert!(!managed_route_destination(200, 100, None));
    }

    /// A `pw-dump` answer holding only the equalizer node with all ten bands.
    fn equalizer_dump(serial: u64, gains: [f32; 10]) -> String {
        let params = crate::equalizer::BANDS_HZ
            .iter()
            .zip(gains)
            .enumerate()
            .flat_map(|(index, (frequency, gain))| {
                vec![
                    serde_json::json!(format!("eq_band_{index}:Freq")),
                    serde_json::json!(frequency),
                    serde_json::json!(format!("eq_band_{index}:Gain")),
                    serde_json::json!(gain),
                ]
            })
            .collect::<Vec<_>>();
        serde_json::json!([{
            "id": 105,
            "info": {
                "props": { "object.serial": serial, "node.name": VIRTUAL_SINK },
                "params": { "Props": [ { "params": params } ] }
            }
        }])
        .to_string()
    }

    /// Scripts the full dump that finds the node and every `set-param` of the
    /// ramp; the verification dump is pushed by each test.
    fn script_ramp(runner: &crate::command::ScriptedRunner) {
        runner.push_output(&equalizer_dump(1005, [0.0; 10]));
        for _ in 0..ramp_steps(&[0.0; 10], &[3.0; 10]) {
            runner.push_output("");
        }
    }

    /// Scripts a successful rollback: full dump, one `set-param`, full dump
    /// showing the previous (flat) gains again.
    fn script_rollback(runner: &crate::command::ScriptedRunner) {
        runner.push_output(&equalizer_dump(1005, [0.0; 10]));
        runner.push_output("");
        runner.push_output(&equalizer_dump(1005, [0.0; 10]));
    }

    fn profile_with_gain(gain: f32) -> crate::equalizer::EqualizerProfile {
        let mut profile = crate::equalizer::EqualizerProfile::default();
        for band in &mut profile.bands {
            band.gain_db = gain;
        }
        profile
    }

    fn full_dumps(calls: &[String]) -> usize {
        calls
            .iter()
            .filter(|call| call.as_str() == "pw-dump")
            .count()
    }

    #[test]
    fn ramp_moves_at_most_a_quarter_db_per_audio_cycle() {
        assert_eq!(ramp_steps(&[0.0; 10], &[1.0; 10]), RAMP_STEPS);
        assert_eq!(ramp_steps(&[0.0; 10], &[3.0; 10]), 12);
        assert_eq!(ramp_steps(&[2.0; 10], &[1.9; 10]), RAMP_STEPS);
        assert_eq!(ramp_steps(&[-12.0; 10], &[12.0; 10]), RAMP_MAX_STEPS);
        assert!(RAMP_STEP_INTERVAL >= Duration::from_millis(1024 * 1000 / 48_000));
    }

    #[test]
    fn apply_profile_uses_one_full_dump_and_one_targeted_dump_for_verification() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        script_ramp(&runner);
        runner.push_output(&equalizer_dump(1005, [3.0; 10]));
        let _guard = crate::command::set_runner(runner.clone());

        apply_profile(
            &profile_with_gain(3.0),
            &crate::equalizer::EqualizerProfile::default(),
        )
        .unwrap();

        let calls = runner.command_lines();
        assert_eq!(full_dumps(&calls), 1, "{calls:?}");
        assert_eq!(calls.first().map(String::as_str), Some("pw-dump"));
        let ramp = calls
            .iter()
            .filter(|call| call.starts_with("pw-cli set-param 105 Props"))
            .count();
        assert_eq!(ramp, 12, "0 -> 3 dB in 0.25 dB steps: {calls:?}");
        assert_eq!(calls.last().map(String::as_str), Some("pw-dump 105"));
        assert_eq!(calls.len(), 1 + 12 + 1);
    }

    #[test]
    fn apply_profile_rolls_back_if_node_recreated() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        script_ramp(&runner);
        // Same id, different serial: the node was recreated during the ramp.
        runner.push_output(&equalizer_dump(1006, [3.0; 10]));
        script_rollback(&runner);
        let _guard = crate::command::set_runner(runner.clone());

        let err = apply_profile(
            &profile_with_gain(3.0),
            &crate::equalizer::EqualizerProfile::default(),
        )
        .unwrap_err();

        assert!(err.contains("recreated"), "{err}");
        assert!(err.contains("restored"), "{err}");
        let calls = runner.command_lines();
        assert_eq!(calls[1 + 12], "pw-dump 105");
        assert!(
            calls.last().is_some_and(|call| call == "pw-dump"),
            "{calls:?}"
        );
    }

    #[test]
    fn apply_profile_rolls_back_if_gains_mismatch() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        script_ramp(&runner);
        // Same node, but the DSP still reports the old gains.
        runner.push_output(&equalizer_dump(1005, [0.0; 10]));
        script_rollback(&runner);
        let _guard = crate::command::set_runner(runner.clone());

        let err = apply_profile(
            &profile_with_gain(3.0),
            &crate::equalizer::EqualizerProfile::default(),
        )
        .unwrap_err();

        assert!(err.contains("did not match"), "{err}");
        assert!(err.contains("restored"), "{err}");
        assert_eq!(runner.command_lines()[1 + 12], "pw-dump 105");
    }

    fn ready_spatial_capability() -> crate::spatial::CapabilityReport {
        crate::spatial::CapabilityReport {
            schema: crate::spatial::SCHEMA,
            pipewire_binary_present: true,
            filter_chain_module_present: true,
            hrtf_dataset_present: true,
            dataset: crate::spatial::DatasetReport {
                schema: crate::spatial::DATASET_SCHEMA,
                manifest_path: Some("/dataset/manifest.json".into()),
                hrir_path: Some("/dataset/hrir.wav".into()),
                format: Some(crate::spatial::SUPPORTED_DATASET_FORMAT.into()),
                name: Some("Open test HRIR".into()),
                license: Some("test-only".into()),
                source_url: Some("https://example.invalid/hrir".into()),
                expected_sha256: Some("0".repeat(64)),
                observed_sha256: Some("0".repeat(64)),
                channels: Some(crate::spatial::REQUIRED_HRIR_CHANNELS),
                sample_rate: Some(48_000),
                legal_review_required: true,
                valid: true,
                error: None,
            },
            ready: true,
            error: None,
        }
    }

    fn otherwise_healthy_spatial_graph() -> SpatialHealth<'static> {
        SpatialHealth {
            service_active: true,
            configured: true,
            graph_observable: true,
            input_format_7_1: true,
            output_format_stereo: true,
            target_equalizer_connected: true,
            default_safe: true,
            chat_isolated: true,
            capture_isolated: true,
            routing_healthy: true,
            observation_error: None,
        }
    }

    #[test]
    fn paused_passive_link_still_describes_spatial_topology() {
        let link = serde_json::json!({
            "type": "PipeWire:Interface:Link",
            "info": {
                "output-node-id": 193,
                "input-node-id": 144,
                "state": "paused"
            }
        });

        assert_eq!(link_node_pair(&link), Some((193, 144)));
        assert_eq!(link_endpoints(&link), Some((193, 144, false)));
    }

    #[test]
    fn spatial_health_rejects_links_to_chat_or_capture_nodes() {
        let capability = ready_spatial_capability();

        let mut chat_link = otherwise_healthy_spatial_graph();
        chat_link.chat_isolated = false;
        assert_eq!(
            spatial_error(&chat_link, &capability).as_deref(),
            Some("unsafe spatial route detected to Chat or a capture node")
        );

        let mut capture_link = otherwise_healthy_spatial_graph();
        capture_link.capture_isolated = false;
        assert_eq!(
            spatial_error(&capture_link, &capability).as_deref(),
            Some("unsafe spatial route detected to Chat or a capture node")
        );
    }

    #[test]
    fn healthy_bypass_remains_active_and_reports_its_processing_mode() {
        let capability = ready_spatial_capability();
        // `enabled` is intentionally not a health prerequisite: a ready
        // persistent graph in dry bypass is still observable and safe.
        assert!(spatial_error(&otherwise_healthy_spatial_graph(), &capability).is_none());
        let controls = |wet: f32, dry: f32| {
            serde_json::json!([{
                "id": 600,
                "info": {
                    "params": { "Props": [{ "params": [
                        SPATIAL_WET_CONTROLS[0], wet, SPATIAL_WET_CONTROLS[1], wet,
                        SPATIAL_DRY_CONTROLS[0], dry, SPATIAL_DRY_CONTROLS[1], dry
                    ] }] }
                }
            }])
        };
        let input = Target {
            id: 600,
            object_serial: 1,
            node_name: SPATIAL_SINK_NODE.into(),
        };
        let binaural = controls(1.0, 0.0);
        let bypass = controls(0.0, 1.0);
        let transitioning = controls(0.5, 0.5);
        assert_eq!(
            spatial_processing_mode(binaural.as_array().unwrap(), Some(&input)),
            "binaural"
        );
        assert_eq!(
            spatial_processing_mode(bypass.as_array().unwrap(), Some(&input)),
            "bypass"
        );
        assert_eq!(
            spatial_processing_mode(transitioning.as_array().unwrap(), Some(&input)),
            "transitioning"
        );
    }

    #[test]
    fn spatial_route_guard_classifies_chat_and_capture_nodes() {
        let chat = node(
            200,
            2_000,
            serde_json::json!({
                "media.class": "Audio/Sink",
                "node.name": "alsa_output.usb_quantum.chat"
            }),
        );
        assert_eq!(node_is_chat_or_capture(&chat), (true, false));

        let microphone = node(
            201,
            2_001,
            serde_json::json!({
                "media.class": "Audio/Source",
                "node.name": "alsa_input.usb_quantum.microphone"
            }),
        );
        assert_eq!(node_is_chat_or_capture(&microphone), (false, true));
    }
    #[test]
    fn unproven_stream_is_not_quarantined_if_previously_routed() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // --- ITERATION 1 ---
        // 1. pactl --format=json list sinks
        runner.push_output(r#"[
            { "name": "alsa_output.usb_quantum.game", "description": "Quantum Game", "index": 200, "properties": { "object.serial": 200, "alsa.components": "USB0ecb:2069", "alsa.device": 0, "api.alsa.pcm.stream": "playback" } },
            { "name": "jambalinux-soniccore-game-equalizer", "index": 100, "properties": { "object.serial": 100 } }
        ]"#);
        // 2. pactl get-default-sink
        runner.push_output("alsa_output.usb_quantum.game");
        // 3. pactl --format=json list sink-inputs (Stream appears on game sink first)
        runner.push_output(r#"[
            { "index": 300, "sink": 200, "properties": { "object.serial": 300, "application.name": "chrome" } }
        ]"#);
        // 4. pactl move-sink-input 300 jambalinux-soniccore-game-equalizer
        runner.push_output("");

        // --- ITERATION 2 ---
        // 1. pactl --format=json list sinks
        runner.push_output(r#"[
            { "name": "alsa_output.usb_quantum.game", "description": "Quantum Game", "index": 200, "properties": { "object.serial": 200, "alsa.components": "USB0ecb:2069", "alsa.device": 0, "api.alsa.pcm.stream": "playback" } },
            { "name": "jambalinux-soniccore-game-equalizer", "index": 100, "properties": { "object.serial": 100 } }
        ]"#);
        // 2. pactl get-default-sink
        runner.push_output("alsa_output.usb_quantum.game");
        // 3. pactl --format=json list sink-inputs (Stream reappears on equalizer sink directly)
        runner.push_output(r#"[
            { "index": 301, "sink": 100, "properties": { "object.serial": 301, "application.name": "chrome" } }
        ]"#);

        let _guard = crate::command::set_runner(runner.clone());
        MOCK_STATE.with(|m| *m.borrow_mut() = Some(BackendState::default()));

        // Run first iteration: Stream 300 is on Game. It will be registered and moved to equalizer.
        run_route_iteration().unwrap();

        let calls = runner.command_lines();
        assert!(
            calls
                .iter()
                .any(|args| args.contains("move-sink-input") && args.contains("300"))
        );

        // Run second iteration: Stream 301 is on Equalizer directly. It should be recognized and NOT quarantined!
        run_route_iteration().unwrap();

        let state = MOCK_STATE.with(|m| m.borrow().clone().unwrap());
        assert!(state.unproven_streams.is_empty(), "Stream was quarantined!");
        assert_eq!(state.routed_streams.len(), 1, "Stream was not registered!");
        assert_eq!(state.routed_streams[0].object_serial, 301);

        // Clean up mock
        MOCK_STATE.with(|m| *m.borrow_mut() = None);
    }

    #[test]
    fn unproven_stream_on_spatial_sink_is_evacuated_safely() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // 1. pactl --format=json list sinks
        runner.push_output(r#"[
            { "name": "alsa_output.usb_quantum.game", "description": "Quantum Game", "index": 200, "properties": { "object.serial": 200, "alsa.components": "USB0ecb:2069", "alsa.device": 0, "api.alsa.pcm.stream": "playback" } },
            { "name": "jambalinux-soniccore-game-equalizer", "index": 100, "properties": { "object.serial": 100 } },
            { "name": "jambalinux-soniccore-game-spatial", "index": 400, "properties": { "object.serial": 400 } }
        ]"#);
        // 2. pw-dump (ensure_equalizer_target)
        runner.push_output(
            r#"[
            {
                "id": 100,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "props": {
                        "node.name": "jambalinux-soniccore-game-equalizer",
                        "media.class": "Audio/Sink",
                        "object.serial": 100
                    }
                }
            }
        ]"#,
        );
        // 3. pactl --format=json list sink-inputs (restore_streams)
        runner.push_output(r#"[
            { "index": 300, "sink": 400, "properties": { "object.serial": 300, "application.name": "chrome" } }
        ]"#);

        // 4. register_spatial_fallback_streams -> pactl list sinks
        runner.push_output(r#"[
            { "name": "alsa_output.usb_quantum.game", "description": "Quantum Game", "index": 200, "properties": { "object.serial": 200, "alsa.components": "USB0ecb:2069", "alsa.device": 0, "api.alsa.pcm.stream": "playback" } },
            { "name": "jambalinux-soniccore-game-equalizer", "index": 100, "properties": { "object.serial": 100 } },
            { "name": "jambalinux-soniccore-game-spatial", "index": 400, "properties": { "object.serial": 400 } }
        ]"#);
        // 5. register_spatial_fallback_streams -> pactl list sink-inputs
        runner.push_output(r#"[
            { "index": 300, "sink": 400, "properties": { "object.serial": 300, "application.name": "chrome" } }
        ]"#);

        // 6. pactl move-sink-input 300 jambalinux-soniccore-game-equalizer
        runner.push_output("");

        let _guard = crate::command::set_runner(runner.clone());
        MOCK_STATE.with(|m| *m.borrow_mut() = Some(BackendState::default()));

        // Call spatial_pipewire::restore_streams() which should evacuate stream 300 to equalizer.
        crate::spatial_pipewire::restore_streams().unwrap();

        let calls = runner.command_lines();
        assert!(calls.iter().any(|args| args.contains("move-sink-input")
            && args.contains("300")
            && args.contains("jambalinux-soniccore-game-equalizer")));

        // The stream should NOT be blindly registered as Game!
        let state = MOCK_STATE.with(|m| m.borrow().clone().unwrap());
        assert!(
            state.routed_streams.is_empty(),
            "Stream was blindly registered!"
        );

        // Clean up mock
        MOCK_STATE.with(|m| *m.borrow_mut() = None);
    }
}
