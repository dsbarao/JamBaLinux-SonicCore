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
use crate::spatial::graph::{SPATIAL_OUTPUT_NODE, SPATIAL_SINK_NODE, TARGET_EQUALIZER_SINK};

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
const RAMP_STEPS: u32 = 4;
const RAMP_STEP_INTERVAL: Duration = Duration::from_millis(10);

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
    enabled: bool,
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

fn command_output(program: &str, args: &[&str]) -> Result<Output, String> {
    Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("could not start {program}: {error}"))
}

fn command_failure(program: &str, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        format!("{program} exited with {}", output.status)
    } else {
        format!("{program} failed: {stderr}")
    }
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

fn read_state() -> Result<Option<BackendState>, String> {
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
    if !health.enabled {
        return Some("the spatial gate is disabled".into());
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

    match pw_dump() {
        Ok(nodes) => {
            input = single_named_node(&nodes, SPATIAL_SINK_NODE);
            output = single_named_node(&nodes, SPATIAL_OUTPUT_NODE);
            target = single_named_audio_sink(&nodes, TARGET_EQUALIZER_SINK);
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
        enabled,
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
            state.routed_streams.push(RoutedStream {
                object_serial: input.object_serial,
                stream_key: input.stream_key.clone(),
                original_sink_name: game.name.clone(),
                original_sink_serial: game.object_serial,
            });
        }
    }
    write_state(&state)
}

fn safe_default_sink<'a>(sinks: &'a [PulseSink], saved: Option<&str>) -> Option<&'a PulseSink> {
    saved
        .and_then(|name| {
            sinks
                .iter()
                .find(|sink| sink.name == name && sink.name != VIRTUAL_SINK)
        })
        .or_else(|| sinks.iter().find(|sink| sink.name != VIRTUAL_SINK))
}

fn run_route_iteration() -> Result<(), String> {
    let _route_lock = route_lock()?;
    let sinks_value = pactl_json(&["list", "sinks"])?;
    let sinks = pulse_sinks(&sinks_value)?;
    let default = pactl_default_sink()?;
    let mut state = read_state()?.unwrap_or_default();
    let mut changed = false;

    if state.routing_paused {
        return Ok(());
    }

    if default == VIRTUAL_SINK {
        let fallback = safe_default_sink(&sinks, state.default_sink_before.as_deref()).ok_or(
            "the virtual equalizer is the default sink and no physical fallback is available",
        )?;
        pactl_success(&["set-default-sink", &fallback.name])?;
    } else if sinks
        .iter()
        .any(|sink| sink.name == default && sink.name != VIRTUAL_SINK)
        && state.default_sink_before.as_deref() != Some(default.as_str())
    {
        state.default_sink_before = Some(default);
        changed = true;
    }
    if changed {
        write_state(&state)?;
        changed = false;
    }

    // Default recovery must not depend on the headset being present. Only
    // after it is safe do we require the exact, confirmed Game sink.
    let game = pulse_game_sink(&sinks_value)?;
    let virtual_sink = sinks
        .iter()
        .find(|sink| sink.name == VIRTUAL_SINK)
        .cloned()
        .ok_or("the automatic equalizer sink is not available yet")?;
    if state.target_node_name != game.name || state.target_object_serial != Some(game.object_serial)
    {
        state.target_node_name.clone_from(&game.name);
        state.target_object_serial = Some(game.object_serial);
        changed = true;
    }

    let inputs = pulse_inputs(&pactl_json(&["list", "sink-inputs"])?)?;
    let spatial_sink_serial = sinks
        .iter()
        .find(|sink| sink.name == SPATIAL_SINK_NODE)
        .map(|sink| sink.object_serial);
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
    state.unproven_streams.retain(|identity| {
        inputs.iter().any(|input| {
            input.object_serial == identity.object_serial && input.stream_key == identity.stream_key
        })
    });
    changed |= state.unproven_streams.len() != old_unproven_len;

    let evacuation_sink = safe_default_sink(&sinks, state.default_sink_before.as_deref())
        .ok_or("no physical sink is available for an unproven equalizer stream")?;
    for input in &inputs {
        let registered = state.routed_streams.iter().any(|route| {
            route.object_serial == input.object_serial && route.stream_key == input.stream_key
        });
        if input.sink_serial == virtual_sink.object_serial
            && !is_equalizer_output(input)
            && !is_spatial_output(input)
            && !registered
        {
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
                write_state(&state)?;
                changed = true;
            }
            // A stream is allowed through the virtual sink only after it was
            // observed on the exact Game endpoint and recorded below.
            pactl_success(&[
                "move-sink-input",
                &input.object_serial.to_string(),
                &evacuation_sink.name,
            ])?;
        }
    }

    for input in inputs {
        if input.sink_serial != game.object_serial || is_equalizer_output(&input) {
            continue;
        }
        if state.unproven_streams.iter().any(|identity| {
            identity.object_serial == input.object_serial && identity.stream_key == input.stream_key
        }) {
            continue;
        }
        if state.routed_streams.iter().any(|route| {
            route.object_serial == input.object_serial && route.stream_key == input.stream_key
        }) {
            continue;
        }
        state.routed_streams.push(RoutedStream {
            object_serial: input.object_serial,
            stream_key: input.stream_key.clone(),
            original_sink_name: game.name.clone(),
            original_sink_serial: game.object_serial,
        });
        // Persist the proven original destination before changing the route.
        write_state(&state)?;
        pactl_success(&[
            "move-sink-input",
            &input.object_serial.to_string(),
            &virtual_sink.name,
        ])?;
        changed = true;
    }
    if state.last_route_error.take().is_some() {
        changed = true;
    }
    if changed {
        write_state(&state)?;
    }
    Ok(())
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
    Command::new("/usr/bin/pipewire")
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

fn ramp_controls(node_id: u32, from: &[f32], to: &[f32]) -> Result<(), String> {
    if from.len() != BANDS_HZ.len() || to.len() != BANDS_HZ.len() {
        return Err("the PipeWire ramp requires exactly ten bands".into());
    }
    for step in 1..=RAMP_STEPS {
        let alpha = step as f32 / RAMP_STEPS as f32;
        let gains = from
            .iter()
            .zip(to)
            .map(|(start, end)| start + ((end - start) * alpha))
            .collect::<Vec<_>>();
        update_controls(node_id, &gains)?;
        if step < RAMP_STEPS {
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
    let default_safe = pactl_default_sink()? != VIRTUAL_SINK;
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
    let state = read_state()?;
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
    if default == VIRTUAL_SINK {
        let fallback = safe_default_sink(&sinks, state.default_sink_before.as_deref()).ok_or(
            "the virtual equalizer is the default sink and no physical fallback is available",
        )?;
        pactl_success(&["set-default-sink", &fallback.name])?;
    } else if sinks
        .iter()
        .any(|sink| sink.name == default && sink.name != VIRTUAL_SINK)
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

    let after = match pw_dump() {
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
    let virtual_sink_serial = sinks
        .iter()
        .find(|sink| sink.name == VIRTUAL_SINK)
        .map(|sink| sink.object_serial);
    let inputs = pulse_inputs(&pactl_json(&["list", "sink-inputs"])?)?;
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
        let original = sinks
            .iter()
            .find(|sink| sink.name == route.original_sink_name)
            .or_else(|| {
                sinks
                    .iter()
                    .find(|sink| sink.object_serial == route.original_sink_serial)
            });
        if let Some(original) = original {
            pactl_success(&[
                "move-sink-input",
                &input.object_serial.to_string(),
                &original.name,
            ])?;
        } else {
            // Keep the proof and original destination for a later reconnect.
            remaining_routes.push(route.clone());
        }
    }
    if pactl_default_sink()? == VIRTUAL_SINK
        && let Some(default) = safe_default_sink(&sinks, state.default_sink_before.as_deref())
    {
        pactl_success(&["set-default-sink", &default.name])?;
    }
    state.routed_streams = remaining_routes;
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
    fn safe_default_never_selects_the_virtual_sink() {
        let sinks = vec![
            PulseSink {
                object_serial: 1,
                name: VIRTUAL_SINK.into(),
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
            enabled: true,
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
}
