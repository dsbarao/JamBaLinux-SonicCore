//! Lifecycle supervisor for the optional spatial PipeWire graph.
//!
//! This backend owns only the short-lived PipeWire process that hosts the
//! spatial filter-chain.  It never starts, stops, or reloads the session
//! PipeWire daemon, WirePlumber, or the equalizer service.  Before removing a
//! live graph it moves every PulseAudio stream still connected to the spatial
//! input to the already-proven equalizer input.  If that proof is absent or
//! ambiguous, it leaves the graph in place rather than guessing a route.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::spatial;
use crate::spatial::graph::{
    SPATIAL_DRY_CONTROLS, SPATIAL_OUTPUT_NODE, SPATIAL_SINK_NODE, SPATIAL_WET_CONTROLS, SpatialMix,
    TARGET_EQUALIZER_SINK, spatial_mix,
};

const CONFIG_NAME: &str = "pipewire-spatial.conf";
const ROUTE_INTERVAL: Duration = Duration::from_millis(500);
const GRAPH_READY_ATTEMPTS: usize = 40;
const GRAPH_READY_INTERVAL: Duration = Duration::from_millis(50);
const MIX_RAMP_STEPS: u32 = 4;
const MIX_RAMP_INTERVAL: Duration = Duration::from_millis(22);
const RESPAWN_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const RESPAWN_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// How many times `stop_graph` re-checks sink-inputs after moving all visible
/// client streams.  Each round issues a fresh `pactl list sink-inputs` and
/// moves anything that appeared since the previous query.  If the sink is
/// still not empty after this many rounds, the graph is kept alive.
const DRAIN_MAX_ATTEMPTS: usize = 5;

/// Interval between drain rounds.  Fifty milliseconds is shorter than the
/// 500 ms PipeWire quantum, so a late-arriving stream that was already being
/// created when the first move completed will be picked up on the next round.
const DRAIN_INTERVAL: Duration = Duration::from_millis(50);

const SPATIAL_OUTPUT_FL: &str = "jambalinux-soniccore-game-spatial-output:output_FL";
const SPATIAL_OUTPUT_FR: &str = "jambalinux-soniccore-game-spatial-output:output_FR";
const EQUALIZER_INPUT_FL: &str = "jambalinux-soniccore-game-equalizer:playback_FL";
const EQUALIZER_INPUT_FR: &str = "jambalinux-soniccore-game-equalizer:playback_FR";

#[derive(Debug, Clone, PartialEq)]
struct PulseSink {
    index: u64,
    name: String,
}

#[derive(Debug, Clone, PartialEq)]
struct PulseInput {
    index: u64,
    sink: u64,
}

/// Every short-lived client runs with the bounded budget from
/// [`crate::command`]. `pw-link --wait` in particular blocks until the link
/// appears, and the supervisor loop below must never be parked on it.
fn command_output(program: &str, args: &[&str]) -> Result<Output, String> {
    crate::command::run_default(program, args)
}

fn command_failure(program: &str, output: &Output) -> String {
    crate::command::failure(program, output)
}

fn pw_dump() -> Result<Vec<Value>, String> {
    let output = command_output("pw-dump", &[])?;
    if !output.status.success() {
        return Err(command_failure("pw-dump", &output));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("pw-dump returned invalid JSON: {error}"))
}

/// The only live controls that change when the spatial gate changes. Keeping
/// the capture sink and its links alive avoids a client-visible device change.
fn mix_payload(mix: SpatialMix) -> String {
    let controls = SPATIAL_WET_CONTROLS
        .into_iter()
        .zip(mix.wet)
        .chain(SPATIAL_DRY_CONTROLS.into_iter().zip(mix.dry))
        .map(|(control, value)| format!("\"{control}\" {value:.3}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{{ params = [ {controls} ] }}")
}

fn spatial_capture_node(nodes: &[Value]) -> Result<Option<&Value>, String> {
    let matches = nodes
        .iter()
        .filter(|node| node_name(node) == Some(SPATIAL_SINK_NODE))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [node] => Ok(Some(*node)),
        _ => Err("multiple spatial capture nodes exist; refusing to change the mix".into()),
    }
}

fn node_id_and_serial(node: &Value) -> Result<(u32, Option<u64>), String> {
    let id = node
        .get("id")
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .ok_or("the spatial capture node has no usable id")?;
    let serial = node
        .get("info")
        .and_then(|info| info.get("props"))
        .and_then(|props| props.get("object.serial"))
        .and_then(Value::as_u64);
    Ok((id, serial))
}

fn set_mix(node_id: u32, from: SpatialMix, to: SpatialMix) -> Result<(), String> {
    let node = node_id.to_string();
    for step in 1..=MIX_RAMP_STEPS {
        let alpha = step as f32 / MIX_RAMP_STEPS as f32;
        let interpolate = |start: f32, end: f32| start + ((end - start) * alpha);
        let mix = SpatialMix {
            wet: [
                interpolate(from.wet[0], to.wet[0]),
                interpolate(from.wet[1], to.wet[1]),
            ],
            dry: [
                interpolate(from.dry[0], to.dry[0]),
                interpolate(from.dry[1], to.dry[1]),
            ],
        };
        let payload = mix_payload(mix);
        let output = command_output("pw-cli", &["set-param", &node, "Props", &payload])?;
        if !output.status.success() {
            return Err(command_failure("pw-cli set-param", &output));
        }
        if step < MIX_RAMP_STEPS {
            thread::sleep(MIX_RAMP_INTERVAL);
        }
    }
    Ok(())
}

/// Applies a wet/dry transition to an existing graph only. `target` is read
/// only after taking the mix lock: the supervisor must not use a gate value
/// sampled before its expensive readiness preflight while a foreground CLI
/// command has since persisted the opposite value.
fn set_mix_if_graph_exists_with_target(
    target: impl FnOnce() -> Result<bool, String>,
) -> Result<bool, String> {
    // The CLI and the supervisor are separate processes.  Keep this guard for
    // the entire read/ramp/verify transaction rather than merely around each
    // `pw-cli` invocation, otherwise their individual ramp steps can
    // interleave and produce an audible bounce to stale intent.
    let _mix_lock = spatial::mix_lock()?;
    let enabled = target()?;
    let nodes = pw_dump()?;
    let Some(node) = spatial_capture_node(&nodes)? else {
        return Ok(false);
    };
    let (node_id, serial) = node_id_and_serial(node)?;
    let from = spatial_mix(node)?;
    let target = SpatialMix::target(enabled);
    if from.matches(target) {
        return Ok(true);
    }
    set_mix(node_id, from, target)?;

    // Props are sent to the filter-chain capture sink, matching the equalizer
    // path. Re-read the exact node: a pw-cli success is not proof that this
    // PipeWire build accepted the live controls.
    let after = pw_dump()?;
    let verified = spatial_capture_node(&after)?
        .filter(|after| node_id_and_serial(after).ok() == Some((node_id, serial)))
        .ok_or("the spatial capture node changed during the live mix update")?;
    if spatial_mix(verified)?.matches(target) {
        Ok(true)
    } else {
        Err("the spatial live controls do not match the requested mix".into())
    }
}

/// Applies the explicitly saved CLI gate to an existing graph only. It never
/// moves a stream, changes a link, or starts/stops a process, so it is safe on
/// the CLI's immediate toggle path.
pub fn set_mix_if_graph_exists() -> Result<bool, String> {
    set_mix_if_graph_exists_with_target(|| spatial::load().map(|profile| profile.enabled))
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

fn pactl_success(args: &[&str]) -> Result<(), String> {
    let output = command_output("pactl", args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failure("pactl", &output))
    }
}

fn pw_link_success(output_port: &str, input_port: &str) -> Result<(), String> {
    let output = command_output("pw-link", &["--wait", output_port, input_port])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failure("pw-link", &output))
    }
}

fn pw_unlink(output_port: &str, input_port: &str) {
    let _ = command_output("pw-link", &["--disconnect", output_port, input_port]);
}

/// Connects the spatial stereo output only to the proven equalizer input.
///
/// WirePlumber may ignore `target.object` on a filter-chain output and fall
/// back to the physical default. The graph disables autoconnect, so these two
/// explicit links are the only authorized output path. A partial connection is
/// rolled back before the error is returned.
fn connect_spatial_output() -> Result<(), String> {
    ensure_equalizer_target()?;
    pw_link_success(SPATIAL_OUTPUT_FL, EQUALIZER_INPUT_FL)?;
    if let Err(error) = pw_link_success(SPATIAL_OUTPUT_FR, EQUALIZER_INPUT_FR) {
        pw_unlink(SPATIAL_OUTPUT_FL, EQUALIZER_INPUT_FL);
        return Err(error);
    }
    Ok(())
}

fn node_name(node: &Value) -> Option<&str> {
    (node.get("type")?.as_str()? == "PipeWire:Interface:Node").then_some(())?;
    node.get("info")?.get("props")?.get("node.name")?.as_str()
}

fn audio_sink_named(nodes: &[Value], name: &str) -> Result<(), String> {
    let matches = nodes
        .iter()
        .filter(|node| {
            node_name(node) == Some(name)
                && node
                    .get("info")
                    .and_then(|info| info.get("props"))
                    .and_then(|props| props.get("media.class"))
                    .and_then(Value::as_str)
                    == Some("Audio/Sink")
        })
        .count();
    match matches {
        1 => Ok(()),
        0 => Err(format!(
            "the required equalizer sink `{name}` is absent; refusing to start spatial processing"
        )),
        _ => Err(format!(
            "multiple compatible equalizer sinks named `{name}` were found; refusing to choose a target"
        )),
    }
}

fn ensure_equalizer_target() -> Result<(), String> {
    audio_sink_named(&pw_dump()?, TARGET_EQUALIZER_SINK)
}

fn pulse_sinks(value: &Value) -> Result<Vec<PulseSink>, String> {
    value
        .as_array()
        .ok_or("pactl list sinks did not return an array")?
        .iter()
        .map(|sink| {
            Ok(PulseSink {
                index: sink
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or("pactl sink is missing its index")?,
                name: sink
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("pactl sink is missing its name")?
                    .to_owned(),
            })
        })
        .collect()
}

fn pulse_inputs(value: &Value) -> Result<Vec<PulseInput>, String> {
    value
        .as_array()
        .ok_or("pactl list sink-inputs did not return an array")?
        .iter()
        .map(|input| {
            Ok(PulseInput {
                index: input
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or("pactl sink input is missing its index")?,
                sink: input
                    .get("sink")
                    .and_then(Value::as_u64)
                    .ok_or("pactl sink input is missing its sink")?,
            })
        })
        .collect()
}

fn exactly_one_sink<'a>(
    sinks: &'a [PulseSink],
    name: &str,
) -> Result<Option<&'a PulseSink>, String> {
    let matches = sinks
        .iter()
        .filter(|sink| sink.name == name)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [sink] => Ok(Some(*sink)),
        _ => Err(format!(
            "multiple PulseAudio sinks named `{name}` were found; refusing to move streams"
        )),
    }
}

/// Moves only streams known to be attached to the spatial capture sink.  No
/// fallback target is permitted: absence or ambiguity of the equalizer leaves
/// the current graph and its streams untouched.
pub fn restore_streams() -> Result<(), String> {
    let (_, _, _) = restore_streams_counted()?;
    Ok(())
}

/// Like [`restore_streams`] but returns `(spatial_index, equalizer_name,
/// moved_count)` so the drain loop in [`stop_graph`] can verify that no
/// client streams remain on the spatial sink without repeating the full sink
/// lookup each round.
fn restore_streams_counted() -> Result<(u64, String, usize), String> {
    let sinks = pulse_sinks(&pactl_json(&["list", "sinks"])?)?;
    let Some(spatial) = exactly_one_sink(&sinks, SPATIAL_SINK_NODE)? else {
        // The caller must not kill until it has proved there are no client
        // inputs; without the sink index that proof is unavailable.
        return Ok((0, String::new(), 0));
    };
    let equalizer = exactly_one_sink(&sinks, TARGET_EQUALIZER_SINK)?.ok_or_else(|| {
        format!(
            "the required equalizer sink `{TARGET_EQUALIZER_SINK}` is absent; refusing to remove the spatial graph"
        )
    })?;
    // Confirm the target in PipeWire as well as PulseAudio. This prevents a
    // stale PulseAudio name from being used as a routing proof.
    ensure_equalizer_target()?;
    let moved = move_spatial_inputs(spatial.index, &equalizer.name)?;
    Ok((spatial.index, equalizer.name.clone(), moved))
}

/// Queries PulseAudio sink-inputs on `spatial_index` and moves each one to
/// `equalizer_name`, registering fallback routes before moving.  Returns the
/// number of streams moved.
fn move_spatial_inputs(spatial_index: u64, equalizer_name: &str) -> Result<usize, String> {
    let inputs = pulse_inputs(&pactl_json(&["list", "sink-inputs"])?)?
        .iter()
        .filter(|input| input.sink == spatial_index)
        .cloned()
        .collect::<Vec<_>>();
    let input_serials = inputs.iter().map(|input| input.index).collect::<Vec<_>>();
    crate::pipewire::register_spatial_fallback_streams(&input_serials)?;
    for input in &inputs {
        pactl_success(&["move-sink-input", &input.index.to_string(), equalizer_name])?;
    }
    Ok(inputs.len())
}

/// Returns the number of PulseAudio sink-inputs currently attached to
/// `spatial_index`.  This is the verification step of the drain loop.
fn client_inputs_on_spatial(spatial_index: u64) -> Result<usize, String> {
    let count = pulse_inputs(&pactl_json(&["list", "sink-inputs"])?)?
        .iter()
        .filter(|input| input.sink == spatial_index)
        .count();
    Ok(count)
}

fn stop_graph(child: &mut Child) -> Result<(), String> {
    // First pass: move every visible client stream from the spatial sink to
    // the equalizer.
    let (spatial_index, equalizer_name, _moved) = restore_streams_counted()?;

    // A vanished PulseAudio sink gives us no index with which to prove the
    // graph has no client inputs.  Do not infer that it is safe to tear down:
    // keep the child alive and let the next supervisor pass observe a stable
    // graph instead.
    if spatial_index == 0 && equalizer_name.is_empty() {
        return Err(
            "the spatial sink disappeared before its client inputs could be confirmed empty; keeping the graph alive"
                .into(),
        );
    }

    // Drain loop: re-check that the spatial sink is truly empty.  A stream
    // can arrive between the initial listing and now (the H2 window); if so,
    // move it and re-check.  Only kill the graph when the sink has zero
    // client streams.
    for attempt in 0..DRAIN_MAX_ATTEMPTS {
        let remaining = client_inputs_on_spatial(spatial_index)?;
        if remaining == 0 {
            child
                .kill()
                .map_err(|error| format!("could not stop the spatial PipeWire graph: {error}"))?;
            child
                .wait()
                .map_err(|error| format!("could not reap the spatial PipeWire graph: {error}"))?;
            return Ok(());
        }
        // There are still client streams — move them and retry.
        eprintln!(
            "spatial supervisor: drain attempt {}/{}: {} stream(s) still on spatial sink, moving",
            attempt + 1,
            DRAIN_MAX_ATTEMPTS,
            remaining
        );
        move_spatial_inputs(spatial_index, &equalizer_name)?;
        thread::sleep(DRAIN_INTERVAL);
    }

    // Could not converge: keep the graph alive so the streams are not
    // orphaned.
    Err(format!(
        "spatial sink still has client streams after {} drain attempts; keeping the graph alive",
        DRAIN_MAX_ATTEMPTS
    ))
}

fn config_path() -> Result<PathBuf, String> {
    Ok(spatial::config_directory()?.join(CONFIG_NAME))
}

fn write_config(contents: &str) -> Result<PathBuf, String> {
    let path = config_path()?;
    let directory = path
        .parent()
        .ok_or("invalid spatial PipeWire configuration path")?;
    fs::create_dir_all(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("{}: {error}", temporary.display()))?;
    use std::io::Write as _;
    if let Err(error) = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
    {
        let _ = fs::remove_file(&temporary);
        return Err(format!("{}: {error}", temporary.display()));
    }
    drop(file);
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("{}: {error}", path.display()));
    }
    Ok(path)
}

fn desired_config() -> Result<Option<String>, String> {
    let profile = spatial::load()?;
    if profile.mode != spatial::SpatialMode::BinauralStereo {
        return Ok(None);
    }
    let capability = spatial::preflight()?;
    if !capability.ready {
        return Err(capability
            .error
            .unwrap_or_else(|| "spatial capability unavailable".into()));
    }
    ensure_equalizer_target()?;
    let hrir = capability
        .dataset
        .hrir_path
        .ok_or("the validated spatial dataset did not report an HRIR path")?;
    spatial::graph::render_filter_chain_config(Path::new(&hrir), profile.enabled).map(Some)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildState {
    Absent,
    Running,
    Exited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupervisorAction {
    Spawn,
    Keep,
    SetMix,
    Teardown,
}

/// Rate-limits replacement graphs after an unexpected child exit. A graph
/// that survives the maximum retry window is considered established. This
/// prevents a graph that crashes after one or two supervisor passes from
/// repeatedly recreating sinks and promoting streams at the initial delay.
#[derive(Debug, Default)]
struct RespawnBackoff {
    consecutive_exits: u32,
    retry_after: Option<Instant>,
    spawned_at: Option<Instant>,
}

fn respawn_delay(consecutive_exits: u32) -> Duration {
    let exponent = consecutive_exits.saturating_sub(1).min(5);
    RESPAWN_BACKOFF_INITIAL
        .checked_mul(1_u32 << exponent)
        .unwrap_or(RESPAWN_BACKOFF_MAX)
        .min(RESPAWN_BACKOFF_MAX)
}

impl RespawnBackoff {
    fn record_exit(&mut self) -> Duration {
        self.consecutive_exits = self.consecutive_exits.saturating_add(1);
        let delay = respawn_delay(self.consecutive_exits);
        self.retry_after = Some(Instant::now() + delay);
        delay
    }

    fn ready(&self) -> bool {
        self.retry_after.is_none_or(|when| Instant::now() >= when)
    }

    fn reset(&mut self) {
        self.consecutive_exits = 0;
        self.retry_after = None;
        self.spawned_at = None;
    }

    fn record_spawn(&mut self) {
        self.record_spawn_at(Instant::now());
    }

    fn reset_if_stable(&mut self) {
        self.reset_if_stable_at(Instant::now());
    }

    fn record_spawn_at(&mut self, now: Instant) {
        self.spawned_at = Some(now);
    }

    fn reset_if_stable_at(&mut self, now: Instant) {
        if self
            .spawned_at
            .is_some_and(|spawned| now.duration_since(spawned) >= RESPAWN_BACKOFF_MAX)
        {
            self.reset();
        }
    }
}

/// Pure lifecycle decision: readiness and selected mode own the graph; the
/// gate owns only wet/dry controls. The running-graph branch intentionally
/// requests a reconciliation on every pass. `set_mix_if_graph_exists`
/// observes the controls first and is a no-op when they already match, which
/// lets the supervisor repair a failed CLI update without replaying a ramp.
fn next_action(
    child: ChildState,
    profile: &spatial::SpatialProfile,
    ready: bool,
) -> SupervisorAction {
    if profile.mode != spatial::SpatialMode::BinauralStereo || !ready {
        return if child == ChildState::Running {
            SupervisorAction::Teardown
        } else {
            SupervisorAction::Keep
        };
    }
    match child {
        ChildState::Absent | ChildState::Exited => SupervisorAction::Spawn,
        ChildState::Running => SupervisorAction::SetMix,
    }
}

fn spawn_pipewire(config: &Path) -> Result<Child, String> {
    Command::new(spatial::pipewire_binary()?)
        .arg("-c")
        .arg(config)
        .spawn()
        .map_err(|error| format!("could not start the spatial PipeWire graph: {error}"))
}

fn spawn_connected_pipewire(config: &Path) -> Result<Child, String> {
    let mut child = spawn_pipewire(config)?;
    for _ in 0..GRAPH_READY_ATTEMPTS {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("could not monitor the spatial PipeWire graph: {error}"))?
        {
            return Err(format!("spatial PipeWire graph exited with {status}"));
        }
        if spatial_graph_exists()? {
            if let Err(error) = connect_spatial_output() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "could not connect the spatial output to the equalizer: {error}"
                ));
            }
            return Ok(child);
        }
        thread::sleep(GRAPH_READY_INTERVAL);
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("the spatial PipeWire graph did not become observable in time".into())
}

fn spatial_graph_exists() -> Result<bool, String> {
    Ok(pw_dump()?.iter().any(|node| {
        matches!(
            node_name(node),
            Some(SPATIAL_SINK_NODE | SPATIAL_OUTPUT_NODE)
        )
    }))
}

/// Reconcile intent with the observable controls. A command failure is not a
/// lifecycle failure: retaining the child is essential because it may carry
/// streams, and the next supervisor pass can retry after PipeWire recovers.
fn reconcile_mix_with_target(target: impl FnOnce() -> Result<bool, String>) {
    // Unlike the immediate CLI path, the supervisor can have spent a long
    // time in preflight since it chose SetMix. Re-read the persisted gate
    // inside the same lock that serializes ramps so it cannot restore an old
    // wet/dry target after a newer CLI toggle has completed.
    match set_mix_if_graph_exists_with_target(target) {
        Ok(true) => {}
        Ok(false) => eprintln!("spatial supervisor: graph disappeared before its mix was updated"),
        Err(error) => eprintln!("spatial supervisor: could not update live mix: {error}"),
    }
}

fn reconcile_mix_from_profile(
    load_profile: impl FnOnce() -> Result<spatial::SpatialProfile, String>,
) {
    reconcile_mix_with_target(|| load_profile().map(|profile| profile.enabled));
}

fn reconcile_mix() {
    reconcile_mix_from_profile(spatial::load);
}

/// Persistent systemd-service entrypoint. The graph is a child process owned
/// by this supervisor. Its lifecycle depends only on the explicit spatial
/// profile and a fresh, fail-closed readiness/target check.
pub fn run() -> Result<(), String> {
    let mut child: Option<Child> = None;
    let mut respawn_backoff = RespawnBackoff::default();
    loop {
        let profile = spatial::load();
        let desired = desired_config();
        let ready = desired.as_ref().is_ok_and(|config| config.is_some());
        let mut exited_status = None;
        let child_state = match child.as_mut() {
            None => ChildState::Absent,
            Some(active) => match active.try_wait() {
                Ok(Some(status)) => {
                    respawn_backoff.reset_if_stable();
                    exited_status = Some(status);
                    ChildState::Exited
                }
                Ok(None) => {
                    respawn_backoff.reset_if_stable();
                    ChildState::Running
                }
                Err(error) => {
                    return Err(format!(
                        "could not monitor the spatial PipeWire graph: {error}"
                    ));
                }
            },
        };
        if let Some(status) = exited_status {
            child = None;
            let delay = respawn_backoff.record_exit();
            eprintln!(
                "spatial supervisor: spatial PipeWire graph exited with {status}; retrying in {} seconds",
                delay.as_secs_f32()
            );
        }
        let action = match profile.as_ref() {
            Ok(profile) => next_action(child_state, profile, ready),
            Err(_) if child_state == ChildState::Running => SupervisorAction::Teardown,
            Err(_) => SupervisorAction::Keep,
        };
        match (action, desired, profile) {
            (SupervisorAction::Spawn, Ok(Some(config)), Ok(_profile))
                if respawn_backoff.ready() =>
            {
                if spatial_graph_exists()? {
                    return Err(
                        "a spatial graph already exists; refusing to create a second graph".into(),
                    );
                }
                child = Some(spawn_connected_pipewire(&write_config(&config)?)?);
                respawn_backoff.record_spawn();
                // The renderer already initializes the graph from its gate.
                // Reconcile anyway in case intent changed while it spawned;
                // a transient command failure never sacrifices the graph.
                reconcile_mix();
            }
            (SupervisorAction::SetMix, _, _) => {
                // This observes the live controls before ramping. It makes a
                // CLI transition idempotent for the supervisor rather than
                // replaying a second wet/dry ramp 500 ms later.
                reconcile_mix();
            }
            (SupervisorAction::Teardown, _, _) => {
                let Some(active) = child.as_mut() else {
                    unreachable!()
                };
                // A drain failure is deliberately non-fatal. Dropping the
                // child here would orphan a graph that can still carry client
                // streams; retain it and retry from a fresh PulseAudio view.
                if let Err(error) = stop_graph(active) {
                    eprintln!("spatial supervisor: {error}");
                } else {
                    child = None;
                    respawn_backoff.reset();
                }
            }
            (SupervisorAction::Keep, Err(error), _) | (SupervisorAction::Keep, _, Err(error)) => {
                eprintln!("spatial supervisor: {error}")
            }
            (SupervisorAction::Keep, _, _) => {}
            _ => {}
        }
        thread::sleep(ROUTE_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const SINKS: &str = r#"[
        { "name": "alsa_output.usb_quantum.game", "index": 200,
          "properties": { "object.serial": 200, "alsa.components": "USB0ecb:2069", "alsa.device": 0, "api.alsa.pcm.stream": "playback" } },
        { "name": "jambalinux-soniccore-game-equalizer", "index": 100,
          "properties": { "object.serial": 100 } },
        { "name": "jambalinux-soniccore-game-spatial", "index": 400,
          "properties": { "object.serial": 400 } }
    ]"#;

    const EQUALIZER_NODE: &str = r#"[
        { "id": 100, "type": "PipeWire:Interface:Node", "info": { "props": {
            "node.name": "jambalinux-soniccore-game-equalizer", "media.class": "Audio/Sink"
        } } }
    ]"#;

    fn spatial_capture_dump(wet: f32, dry: f32) -> String {
        // Keep an output node with identical Props in the fixture: a test
        // must fail if live controls are accidentally sent to it instead of
        // the capture sink (id 400).
        let props = serde_json::json!([{ "params": [
            SPATIAL_WET_CONTROLS[0], wet, SPATIAL_WET_CONTROLS[1], wet,
            SPATIAL_DRY_CONTROLS[0], dry, SPATIAL_DRY_CONTROLS[1], dry
        ] }]);
        serde_json::json!([
            {
                "id": 400,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "props": {
                        "node.name": SPATIAL_SINK_NODE,
                        "media.class": "Audio/Sink",
                        "object.serial": 400
                    },
                    "params": { "Props": props }
                }
            },
            {
                "id": 401,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "props": {
                        "node.name": SPATIAL_OUTPUT_NODE,
                        "media.class": "Audio/Source",
                        "object.serial": 401
                    },
                    "params": { "Props": props }
                }
            }
        ])
        .to_string()
    }

    fn profile(mode: spatial::SpatialMode, enabled: bool) -> spatial::SpatialProfile {
        spatial::SpatialProfile {
            schema: spatial::SCHEMA,
            mode,
            enabled,
        }
    }

    fn inputs(entries: &[(u64, u64)]) -> String {
        let entries = entries
            .iter()
            .map(|(index, sink)| {
                format!(
                    r#"{{ "index": {index}, "sink": {sink}, "properties": {{ "object.serial": {index}, "application.name": "chrome" }} }}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("[{entries}]")
    }

    fn queue_move(runner: &crate::command::ScriptedRunner, visible_inputs: &[(u64, u64)]) {
        // register_spatial_fallback_streams must run before every move, even
        // when WirePlumber placed an otherwise-unregistered stream here.
        runner.push_output(SINKS);
        runner.push_output(&inputs(visible_inputs));
        runner.push_output("");
    }

    fn live_child() -> Child {
        Command::new("sh")
            .args(["-c", "exec sleep 60"])
            .spawn()
            .expect("start disposable child")
    }

    fn reap(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    fn assert_registration_precedes_every_move(calls: &[String]) {
        let moves = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| call.starts_with("pactl move-sink-input"))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        assert!(!moves.is_empty());
        for move_index in moves {
            assert_eq!(calls[move_index - 2], "pactl --format=json list sinks");
            assert_eq!(
                calls[move_index - 1],
                "pactl --format=json list sink-inputs"
            );
        }
    }

    #[test]
    fn next_action_keeps_the_graph_for_gate_toggles_and_tears_it_down_only_when_required() {
        let binaural_on = profile(spatial::SpatialMode::BinauralStereo, true);
        let binaural_off = profile(spatial::SpatialMode::BinauralStereo, false);
        let mode_off = profile(spatial::SpatialMode::Off, false);

        // off -> on and on -> off alter only the live mix of a running graph.
        assert_eq!(
            next_action(ChildState::Running, &binaural_on, true),
            SupervisorAction::SetMix
        );
        assert_eq!(
            next_action(ChildState::Running, &binaural_off, true),
            SupervisorAction::SetMix
        );
        // A mode change or lost readiness removes a live graph. If it is not
        // safe to restore streams, stop_graph retains the child and retries;
        // this decision remains Teardown on the next pass.
        assert_eq!(
            next_action(ChildState::Running, &mode_off, true),
            SupervisorAction::Teardown
        );
        assert_eq!(
            next_action(ChildState::Running, &binaural_on, false),
            SupervisorAction::Teardown
        );
        assert_eq!(
            next_action(ChildState::Absent, &binaural_on, false),
            SupervisorAction::Keep
        );
        // A mode-off profile cannot create a graph, whereas a dead ready child
        // is recreated under the same selected binaural mode.
        assert_eq!(
            next_action(ChildState::Absent, &mode_off, true),
            SupervisorAction::Keep
        );
        assert_eq!(
            next_action(ChildState::Exited, &binaural_off, true),
            SupervisorAction::Spawn
        );
    }

    #[test]
    fn live_mix_updates_issue_only_pw_dump_and_pw_cli_without_routing() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // Enable observes dry, ramps, then verifies wet on the capture node.
        runner.push_output(&spatial_capture_dump(0.0, 1.0));
        for _ in 0..MIX_RAMP_STEPS {
            runner.push_output("");
        }
        runner.push_output(&spatial_capture_dump(1.0, 0.0));
        // Disable follows the same route-free sequence and verifies dry.
        runner.push_output(&spatial_capture_dump(1.0, 0.0));
        for _ in 0..MIX_RAMP_STEPS {
            runner.push_output("");
        }
        runner.push_output(&spatial_capture_dump(0.0, 1.0));
        let _guard = crate::command::set_runner(runner.clone());

        assert!(set_mix_if_graph_exists_with_target(|| Ok(true)).unwrap());
        assert!(set_mix_if_graph_exists_with_target(|| Ok(false)).unwrap());
        let calls = runner.command_lines();
        assert!(
            calls
                .iter()
                .all(|call| call.starts_with("pw-dump") || call.starts_with("pw-cli set-param")),
            "unexpected transition command: {calls:?}"
        );
        assert!(
            !calls
                .iter()
                .any(|call| call.contains("move-sink-input") || call.contains("pw-link"))
        );
        assert!(
            calls
                .iter()
                .any(|call| call.contains("wetDryL:Gain 1\" 1.000"))
        );
        assert!(
            calls
                .iter()
                .any(|call| call.contains("wetDryL:Gain 2\" 1.000"))
        );
        assert!(
            calls
                .iter()
                .filter(|call| call.starts_with("pw-cli set-param"))
                .all(|call| call.starts_with("pw-cli set-param 400 Props")),
            "the live controls must target capture node 400, not output node 401: {calls:?}"
        );
    }

    #[test]
    fn supervisor_retries_a_failed_mix_on_its_next_pass() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // First pass sees wet controls but PipeWire rejects the first ramp
        // step. The error is logged and leaves the graph intact.
        runner.push_output(&spatial_capture_dump(1.0, 0.0));
        runner.push_failure(1, "PipeWire is restarting");
        // The next pass must make a fresh observation and send the entire
        // wet-to-dry ramp, then verify the resulting dry controls.
        runner.push_output(&spatial_capture_dump(1.0, 0.0));
        for _ in 0..MIX_RAMP_STEPS {
            runner.push_output("");
        }
        runner.push_output(&spatial_capture_dump(0.0, 1.0));
        let _guard = crate::command::set_runner(runner.clone());

        reconcile_mix_from_profile(|| Ok(profile(spatial::SpatialMode::BinauralStereo, false)));
        let commands_after_failed_pass = runner.command_lines().len();
        reconcile_mix_with_target(|| Ok(false));

        let retry_commands = &runner.command_lines()[commands_after_failed_pass..];
        assert_eq!(
            retry_commands
                .iter()
                .filter(|call| call.starts_with("pw-cli set-param"))
                .count(),
            MIX_RAMP_STEPS as usize,
            "the next supervisor pass must replay the full dry ramp"
        );
        assert!(retry_commands.last().is_some_and(|call| call == "pw-dump"));
    }

    #[test]
    fn respawn_backoff_grows_exponentially_and_is_capped() {
        assert_eq!(respawn_delay(1), Duration::from_secs(1));
        assert_eq!(respawn_delay(2), Duration::from_secs(2));
        assert_eq!(respawn_delay(3), Duration::from_secs(4));
        assert_eq!(respawn_delay(6), Duration::from_secs(30));
        assert_eq!(respawn_delay(u32::MAX), Duration::from_secs(30));
    }

    #[test]
    fn short_lived_replacement_graphs_keep_increasing_the_respawn_delay() {
        let mut backoff = RespawnBackoff::default();
        let started = Instant::now();

        // Each replacement remains up for only one supervisor interval. That
        // must not erase the crash history, or the supervisor would recreate
        // a client-visible sink every two seconds forever.
        for (exit, expected_delay) in [
            (started + Duration::from_millis(500), Duration::from_secs(1)),
            (started + Duration::from_secs(2), Duration::from_secs(2)),
            (started + Duration::from_secs(5), Duration::from_secs(4)),
        ] {
            backoff.record_spawn_at(exit - Duration::from_millis(500));
            backoff.reset_if_stable_at(exit);
            assert_eq!(backoff.record_exit(), expected_delay);
        }

        // A graph that survives the full maximum backoff is established and
        // intentionally earns a fresh initial retry delay after a later exit.
        backoff.record_spawn_at(started);
        backoff.reset_if_stable_at(started + RESPAWN_BACKOFF_MAX);
        assert_eq!(backoff.record_exit(), RESPAWN_BACKOFF_INITIAL);
    }

    #[test]
    fn respawn_after_a_failed_reconcile_retries_the_new_graph_mix() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // The first reconciliation, immediately after a replacement graph
        // spawns, cannot observe Props yet. The following pass sees the new
        // graph's wet defaults and must apply the disabled gate.
        runner.push_error("pw-dump timed out while PipeWire restarted");
        runner.push_output(&spatial_capture_dump(1.0, 0.0));
        for _ in 0..MIX_RAMP_STEPS {
            runner.push_output("");
        }
        runner.push_output(&spatial_capture_dump(0.0, 1.0));
        let _guard = crate::command::set_runner(runner.clone());

        let binaural_off = profile(spatial::SpatialMode::BinauralStereo, false);
        assert_eq!(
            next_action(ChildState::Exited, &binaural_off, true),
            SupervisorAction::Spawn
        );
        let profile_on_disk = profile(spatial::SpatialMode::BinauralStereo, false);
        reconcile_mix_from_profile(|| Ok(profile_on_disk));

        assert_eq!(
            next_action(ChildState::Running, &binaural_off, true),
            SupervisorAction::SetMix
        );
        reconcile_mix_with_target(|| Ok(false));
        assert_eq!(
            runner
                .calls()
                .iter()
                .filter(|call| call.program == "pw-cli")
                .count(),
            MIX_RAMP_STEPS as usize
        );
    }

    #[test]
    fn supervisor_reconciles_a_failed_cli_enable_from_live_controls() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // A best-effort CLI enable failed before it could inspect the graph.
        runner.push_error("pw-dump timed out");
        // The supervisor observes the dry controls left by the preceding
        // successful disable, then ramps them back to wet.
        runner.push_output(&spatial_capture_dump(0.0, 1.0));
        for _ in 0..MIX_RAMP_STEPS {
            runner.push_output("");
        }
        runner.push_output(&spatial_capture_dump(1.0, 0.0));
        let _guard = crate::command::set_runner(runner.clone());

        assert!(set_mix_if_graph_exists_with_target(|| Ok(true)).is_err());
        let binaural_on = profile(spatial::SpatialMode::BinauralStereo, true);
        assert_eq!(
            next_action(ChildState::Running, &binaural_on, true),
            SupervisorAction::SetMix
        );
        reconcile_mix_with_target(|| Ok(true));

        assert_eq!(
            runner
                .calls()
                .iter()
                .filter(|call| call.program == "pw-cli")
                .count(),
            MIX_RAMP_STEPS as usize
        );
    }

    #[test]
    fn cli_transition_and_supervisor_reconciliation_do_not_double_ramp() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // The CLI performs one wet-to-dry ramp and confirms it.
        runner.push_output(&spatial_capture_dump(1.0, 0.0));
        for _ in 0..MIX_RAMP_STEPS {
            runner.push_output("");
        }
        runner.push_output(&spatial_capture_dump(0.0, 1.0));
        // The following supervisor pass observes dry and emits no Props.
        runner.push_output(&spatial_capture_dump(0.0, 1.0));
        let _guard = crate::command::set_runner(runner.clone());

        set_mix_if_graph_exists_with_target(|| Ok(false)).unwrap();
        reconcile_mix_with_target(|| Ok(false));

        let payloads = runner
            .calls()
            .into_iter()
            .filter_map(|call| {
                (call.program == "pw-cli")
                    .then(|| call.args.last().cloned())
                    .flatten()
            })
            .collect::<Vec<_>>();
        assert_eq!(payloads.len(), MIX_RAMP_STEPS as usize);
        for (expected, payload) in [0.75, 0.5, 0.25, 0.0].into_iter().zip(payloads) {
            assert!(
                payload.contains(&format!("wetDryL:Gain 1\" {expected:.3}")),
                "{payload}"
            );
        }
    }

    #[test]
    fn supervisor_reloads_the_gate_inside_the_mix_lock_before_ramping() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // The supervisor chose SetMix before preflight. While it was
        // busy, the CLI saved false and completed its dry ramp. Its fresh
        // profile read under mix_lock sees false, so dry controls need no
        // pw-cli update.
        runner.push_output(&spatial_capture_dump(0.0, 1.0));
        let _guard = crate::command::set_runner(runner.clone());

        let stale_decision = profile(spatial::SpatialMode::BinauralStereo, true);
        assert_eq!(
            next_action(ChildState::Running, &stale_decision, true),
            SupervisorAction::SetMix
        );
        let profile_on_disk = profile(spatial::SpatialMode::BinauralStereo, false);
        reconcile_mix_from_profile(|| Ok(profile_on_disk));

        assert!(
            runner
                .command_lines()
                .iter()
                .all(|call| call.starts_with("pw-dump"))
        );
    }

    #[test]
    fn immediate_mix_uses_the_gate_read_under_the_lock_not_a_stale_cli_value() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // An enable command may have captured `true` before a newer disable
        // saved false. The immediate path must load false only after it owns
        // mix_lock, see that PipeWire is already dry, and issue no ramp back
        // to wet.
        runner.push_output(&spatial_capture_dump(0.0, 1.0));
        let _guard = crate::command::set_runner(runner.clone());

        let captured_cli_value = true;
        let saved_gate = false;
        assert_ne!(captured_cli_value, saved_gate);
        assert!(set_mix_if_graph_exists_with_target(|| Ok(saved_gate)).unwrap());

        assert!(
            runner
                .command_lines()
                .iter()
                .all(|call| call.starts_with("pw-dump")),
            "a stale captured CLI value must not produce a wet ramp"
        );
    }

    #[test]
    fn lost_readiness_tears_down_after_safely_restoring_late_streams() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // Initial restore sees stream 300.
        runner.push_output(SINKS);
        runner.push_output(EQUALIZER_NODE);
        runner.push_output(&inputs(&[(300, 400)]));
        queue_move(&runner, &[(300, 400)]);
        // The verification read closes H2: a new, unregistered stream appears
        // after the first listing and must also be evacuated.
        runner.push_output(&inputs(&[(301, 400)]));
        // move_spatial_inputs takes its own fresh snapshot before registering
        // and moving that stream.
        runner.push_output(&inputs(&[(301, 400)]));
        queue_move(&runner, &[(301, 400)]);
        // Kill is permitted only after this fresh read observes zero clients.
        runner.push_output(&inputs(&[]));

        let _guard = crate::command::set_runner(runner.clone());
        crate::pipewire::install_mock_state_for_test();
        let mut child = live_child();
        let profile = profile(spatial::SpatialMode::BinauralStereo, true);
        assert_eq!(
            next_action(ChildState::Running, &profile, false),
            SupervisorAction::Teardown
        );
        stop_graph(&mut child).expect("the sink drains after moving the late stream");

        let calls = runner.command_lines();
        assert_registration_precedes_every_move(&calls);
        assert!(calls.iter().any(|call| {
            call == "pactl move-sink-input 300 jambalinux-soniccore-game-equalizer"
        }));
        assert!(calls.iter().any(|call| {
            call == "pactl move-sink-input 301 jambalinux-soniccore-game-equalizer"
        }));
        assert!(child.try_wait().expect("observe child").is_some());
        crate::pipewire::clear_mock_state_for_test();
    }

    #[test]
    fn teardown_keeps_the_graph_alive_when_the_sink_never_drains() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        runner.push_output(SINKS);
        runner.push_output(EQUALIZER_NODE);
        runner.push_output(&inputs(&[(300, 400)]));
        queue_move(&runner, &[(300, 400)]);
        for _ in 0..DRAIN_MAX_ATTEMPTS {
            runner.push_output(&inputs(&[(301, 400)]));
            runner.push_output(&inputs(&[(301, 400)]));
            queue_move(&runner, &[(301, 400)]);
        }

        let _guard = crate::command::set_runner(runner.clone());
        crate::pipewire::install_mock_state_for_test();
        let mut child = live_child();
        let error = stop_graph(&mut child).expect_err("a non-empty sink must not be killed");

        assert!(error.contains("keeping the graph alive"));
        assert!(child.try_wait().expect("observe child").is_none());
        assert_registration_precedes_every_move(&runner.command_lines());
        reap(&mut child);
        crate::pipewire::clear_mock_state_for_test();
    }

    #[test]
    fn lost_readiness_keeps_the_graph_when_restoration_cannot_be_proven() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // The spatial sink exists, but the required EQ sink is absent. There
        // is no safe restoration target, so no move or child kill is allowed.
        runner.push_output(
            r#"[
            { "name": "jambalinux-soniccore-game-spatial", "index": 400,
              "properties": { "object.serial": 400 } }
        ]"#,
        );
        let _guard = crate::command::set_runner(runner.clone());
        let mut child = live_child();

        let profile = profile(spatial::SpatialMode::BinauralStereo, true);
        assert_eq!(
            next_action(ChildState::Running, &profile, false),
            SupervisorAction::Teardown
        );
        let error = stop_graph(&mut child).expect_err("ambiguous restoration must retain graph");

        assert!(error.contains("equalizer sink"));
        assert!(child.try_wait().expect("observe child").is_none());
        assert!(
            !runner
                .command_lines()
                .iter()
                .any(|call| call.contains("move-sink-input"))
        );
        reap(&mut child);
    }
}
