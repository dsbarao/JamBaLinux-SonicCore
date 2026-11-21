//! Lifecycle supervisor for the optional spatial PipeWire graph.
//!
//! This backend owns only the short-lived PipeWire process that hosts the
//! spatial filter-chain.  It never starts, stops, or reloads the session
//! PipeWire daemon, WirePlumber, or the equalizer service.  Before removing a
//! live graph it moves every PulseAudio stream still connected to the spatial
//! input to the already-proven equalizer input.  If that proof is absent or
//! ambiguous, it leaves the graph in place rather than guessing a route.

use std::fs::{self, OpenOptions};
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::spatial;
use crate::spatial::graph::{
    SPATIAL_OUTPUT_NODE, SPATIAL_SINK_NODE, SpatialMix,
    TARGET_EQUALIZER_SINK, spatial_mix,
};

const CONFIG_NAME: &str = "pipewire-spatial.conf";
const ROUTE_INTERVAL: Duration = Duration::from_millis(500);
/// Pass interval while a running graph needed nothing. CLI and widget mix
/// changes are applied by the command itself, so this only delays automatic
/// recovery by half a second while halving the idle polling cost.
const STEADY_INTERVAL: Duration = Duration::from_secs(1);
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
    let controls = mix
        .controls()
        .into_iter()
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

/// Steps for a transition: a small one (a slider being dragged) is applied
/// at once, so the sound follows the hand; a large one (a mode change, the
/// 3D switched on or off) keeps the click-free ramp.
fn ramp_steps(from: SpatialMix, to: SpatialMix) -> u32 {
    let small = from
        .controls()
        .into_iter()
        .zip(to.controls())
        .all(|((name, start), (_, end))| {
            // Shelf gains are in dB; the rest are mixer factors.
            let limit = if name.ends_with(":Gain") { 1.0 } else { 0.15 };
            (end - start).abs() <= limit
        });
    if small { 1 } else { MIX_RAMP_STEPS }
}

fn set_mix(node_id: u32, from: SpatialMix, to: SpatialMix) -> Result<(), String> {
    let node = node_id.to_string();
    let steps = ramp_steps(from, to);
    for step in 1..=steps {
        let alpha = step as f32 / steps as f32;
        let mix = SpatialMix::interpolate(from, to, alpha);
        let payload = mix_payload(mix);
        let output = command_output("pw-cli", &["set-param", &node, "Props", &payload])?;
        if !output.status.success() {
            return Err(command_failure("pw-cli set-param", &output));
        }
        if step < steps {
            thread::sleep(MIX_RAMP_INTERVAL);
        }
    }
    Ok(())
}

/// Applies a wet/dry transition to an existing graph only. `target` is read
/// only after taking the mix lock: the supervisor must not use a gate value
/// sampled before its expensive readiness preflight while a foreground CLI
/// command has since persisted the opposite value.
/// What a caller asks the live controls to become: the bare gate (the
/// built-in levels) or a saved profile with its listening mode.
pub(crate) trait MixIntent {
    fn mix(self) -> SpatialMix;
}

impl MixIntent for bool {
    fn mix(self) -> SpatialMix {
        SpatialMix::target(self)
    }
}

impl MixIntent for spatial::SpatialProfile {
    fn mix(self) -> SpatialMix {
        SpatialMix::for_sound(self.enabled, &self.sound)
    }
}

fn set_mix_if_graph_exists_with_target<I: MixIntent>(
    target: impl FnOnce() -> Result<I, String>,
) -> Result<bool, String> {
    set_mix_if_graph_exists_observed(None, target)
}

/// Like [`set_mix_if_graph_exists_with_target`], but first consults a
/// `pw-dump` snapshot the supervisor already took this pass. When that
/// snapshot already shows the target mix (the steady state) no further
/// `pw-dump` is spent; any mismatch is re-read fresh under the lock before
/// ramping, so a stale snapshot can never drive a ramp.
fn set_mix_if_graph_exists_observed<I: MixIntent>(
    observed: Option<&[Value]>,
    target: impl FnOnce() -> Result<I, String>,
) -> Result<bool, String> {
    // The CLI and the supervisor are separate processes.  Keep this guard for
    // the entire read/ramp/verify transaction rather than merely around each
    // `pw-cli` invocation, otherwise their individual ramp steps can
    // interleave and produce an audible bounce to stale intent.
    let _mix_lock = spatial::mix_lock()?;
    let target = target()?.mix();
    if let Some(objects) = observed {
        match spatial_capture_node(objects)? {
            None => return Ok(false),
            Some(node) if spatial_mix(node)?.matches(target) => {
                return Ok(true);
            }
            Some(_) => {}
        }
    }
    let nodes = pw_dump()?;
    let Some(node) = spatial_capture_node(&nodes)? else {
        return Ok(false);
    };
    let (node_id, serial) = node_id_and_serial(node)?;
    let from = spatial_mix(node)?;
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
    set_mix_if_graph_exists_with_target(spatial::load)
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

/// The two stereo links that carry the spatial output into the equalizer.
const SPATIAL_OUTPUT_LINKS: [(&str, &str); 2] = [
    (SPATIAL_OUTPUT_FL, EQUALIZER_INPUT_FL),
    (SPATIAL_OUTPUT_FR, EQUALIZER_INPUT_FR),
];

/// PipeWire ids of the ports addressed as `node.name:port.name`.
fn port_ids(objects: &[Value], qualified: &str) -> Vec<u64> {
    let Some((node, port)) = qualified.split_once(':') else {
        return Vec::new();
    };
    let node_ids = objects
        .iter()
        .filter(|object| node_name(object) == Some(node))
        .filter_map(|object| object.get("id").and_then(Value::as_u64))
        .collect::<Vec<_>>();
    objects
        .iter()
        .filter(|object| {
            object.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Port")
        })
        .filter(|object| {
            let props = object.get("info").and_then(|info| info.get("props"));
            props
                .and_then(|props| props.get("port.name"))
                .and_then(Value::as_str)
                == Some(port)
                && props
                    .and_then(|props| props.get("node.id"))
                    .and_then(Value::as_u64)
                    .is_some_and(|id| node_ids.contains(&id))
        })
        .filter_map(|object| object.get("id").and_then(Value::as_u64))
        .collect()
}

fn link_exists(objects: &[Value], output_port: &str, input_port: &str) -> bool {
    let outputs = port_ids(objects, output_port);
    let inputs = port_ids(objects, input_port);
    objects.iter().any(|object| {
        object.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Link")
            && object.get("info").is_some_and(|info| {
                info.get("output-port-id")
                    .and_then(Value::as_u64)
                    .is_some_and(|id| outputs.contains(&id))
                    && info
                        .get("input-port-id")
                        .and_then(Value::as_u64)
                        .is_some_and(|id| inputs.contains(&id))
            })
    })
}

/// Links whose ports both exist but are not connected to each other.
fn missing_output_links(objects: &[Value]) -> Vec<(&'static str, &'static str)> {
    SPATIAL_OUTPUT_LINKS
        .into_iter()
        .filter(|(output, input)| {
            !port_ids(objects, output).is_empty()
                && !port_ids(objects, input).is_empty()
                && !link_exists(objects, output, input)
        })
        .collect()
}

/// Re-creates the spatial→equalizer links after the equalizer node was
/// recreated (service restart, PipeWire recovery). The graph disables
/// autoconnect, so without this repair its streams would stay silent.
#[cfg(test)]
fn repair_spatial_output() -> Result<bool, String> {
    repair_spatial_output_observed(&pw_dump()?)
}

fn repair_spatial_output_observed(objects: &[Value]) -> Result<bool, String> {
    let missing = missing_output_links(objects);
    if missing.is_empty() {
        return Ok(false);
    }
    audio_sink_named(objects, TARGET_EQUALIZER_SINK)?;
    for (output, input) in missing {
        pw_link_success(output, input)?;
    }
    Ok(true)
}

/// Stereo upmix for the apps playing into the 3D sink. PipeWire converts each
/// stream to the sink's 7.1 layout in the stream itself, and the default
/// method copies stereo to the front pair only: centre and LFE stay silent,
/// so the Diálogos and Graves levels could not act on stereo games. PSD
/// upmix derives them (and the rears) from the stereo image. Only streams
/// linked to the spatial sink are touched, live, so no PipeWire restart (and
/// no lost microphone in open apps) is needed.
const UPMIX_PAYLOAD: &str = "{ params = [ \"channelmix.upmix\" true \"channelmix.upmix-method\" \"psd\" \"channelmix.lfe-cutoff\" 150.0 \"channelmix.fc-cutoff\" 12000.0 \"channelmix.rear-delay\" 12.0 ] }";

/// Ids of the playback streams linked into the spatial sink whose upmix is
/// not PSD yet.
fn streams_needing_upmix(objects: &[Value]) -> Vec<u32> {
    let Some(sink) = objects
        .iter()
        .find(|node| node_name(node) == Some(SPATIAL_SINK_NODE))
        .and_then(|node| node.get("id").and_then(Value::as_u64))
    else {
        return Vec::new();
    };
    let sources = objects
        .iter()
        .filter(|object| {
            object.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Link")
        })
        .filter_map(|link| {
            let info = link.get("info")?;
            (info.get("input-node-id")?.as_u64()? == sink)
                .then(|| info.get("output-node-id")?.as_u64())
                .flatten()
        })
        .collect::<std::collections::BTreeSet<_>>();
    objects
        .iter()
        .filter_map(|node| {
            let id = node.get("id")?.as_u64()?;
            if !sources.contains(&id) {
                return None;
            }
            let info = node.get("info")?;
            if info.get("props")?.get("media.class")?.as_str()? != "Stream/Output/Audio" {
                return None;
            }
            let method = info
                .get("params")
                .and_then(|params| params.get("Props"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|props| props.get("params").and_then(Value::as_array))
                .find_map(|pairs| {
                    pairs.as_chunks::<2>().0.iter().find_map(|pair| {
                        (pair[0].as_str() == Some("channelmix.upmix-method"))
                            .then(|| pair[1].as_str())
                            .flatten()
                    })
                });
            (method != Some("psd")).then(|| u32::try_from(id).ok()).flatten()
        })
        .collect()
}

/// Best effort: a stream may end between the snapshot and the update.
fn upmix_spatial_streams(objects: &[Value]) {
    for id in streams_needing_upmix(objects) {
        let node = id.to_string();
        match command_output("pw-cli", &["set-param", &node, "Props", UPMIX_PAYLOAD]) {
            Ok(output) if output.status.success() => {}
            Ok(output) => eprintln!(
                "spatial supervisor: could not upmix stream {id}: {}",
                command_failure("pw-cli set-param", &output)
            ),
            Err(error) => eprintln!("spatial supervisor: could not upmix stream {id}: {error}"),
        }
    }
}

fn disconnect_spatial_output() {
    for (output, input) in SPATIAL_OUTPUT_LINKS {
        pw_unlink(output, input);
    }
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
        // If PipeWire itself no longer has the capture node, no client can be
        // attached to it: the child is an empty shell (for example after the
        // session PipeWire restarted) and is safe to stop.
        if spatial_capture_node(&pw_dump()?)?.is_none() {
            return kill_child(child);
        }
        return Err(
            "the spatial sink disappeared before its client inputs could be confirmed empty; keeping the graph alive"
                .into(),
        );
    }

    // While the output stays linked, the equalizer router sees a connected
    // spatial graph and promotes registered streams straight back to it,
    // undoing the drain. Unlink first; the drain below then converges.
    disconnect_spatial_output();
    let result = drain_and_stop(child, spatial_index, &equalizer_name);
    if result.is_err() {
        // The graph is kept because it still carries streams: give them back
        // their path to the headset.
        if let Err(error) = connect_spatial_output() {
            eprintln!("spatial supervisor: could not reconnect the kept graph: {error}");
        }
    }
    result
}

fn kill_child(child: &mut Child) -> Result<(), String> {
    child
        .kill()
        .map_err(|error| format!("could not stop the spatial PipeWire graph: {error}"))?;
    child
        .wait()
        .map_err(|error| format!("could not reap the spatial PipeWire graph: {error}"))?;
    Ok(())
}

fn drain_and_stop(
    child: &mut Child,
    spatial_index: u64,
    equalizer_name: &str,
) -> Result<(), String> {
    // Drain loop: re-check that the spatial sink is truly empty.  A stream
    // can arrive between the initial listing and now (the H2 window); if so,
    // move it and re-check.  Only kill the graph when the sink has zero
    // client streams.
    for attempt in 0..DRAIN_MAX_ATTEMPTS {
        let remaining = client_inputs_on_spatial(spatial_index)?;
        if remaining == 0 {
            return kill_child(child);
        }
        // There are still client streams — move them and retry.
        eprintln!(
            "spatial supervisor: drain attempt {}/{}: {} stream(s) still on spatial sink, moving",
            attempt + 1,
            DRAIN_MAX_ATTEMPTS,
            remaining
        );
        move_spatial_inputs(spatial_index, equalizer_name)?;
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

/// `objects` is this pass's `pw-dump` snapshot; it is only consulted for a
/// binaural profile, so an empty slice is fine when no graph is wanted.
fn desired_config(objects: &[Value]) -> Result<Option<String>, String> {
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
    audio_sink_named(objects, TARGET_EQUALIZER_SINK)?;
    let hrir = capability
        .dataset
        .hrir_path
        .ok_or("the validated spatial dataset did not report an HRIR path")?;
    spatial::graph::render_filter_chain_config_with_sound(
        Path::new(&hrir),
        profile.enabled,
        &profile.sound,
    )
    .map(Some)
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
    match wait_and_connect(&mut child) {
        Ok(()) => Ok(child),
        Err(error) => {
            // Never leave a half-started graph behind: an orphan would make
            // the next spawn refuse with "a spatial graph already exists".
            let _ = child.kill();
            let _ = child.wait();
            Err(error)
        }
    }
}

fn wait_and_connect(child: &mut Child) -> Result<(), String> {
    for _ in 0..GRAPH_READY_ATTEMPTS {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("could not monitor the spatial PipeWire graph: {error}"))?
        {
            return Err(format!("spatial PipeWire graph exited with {status}"));
        }
        if spatial_graph_exists()? {
            return connect_spatial_output().map_err(|error| {
                format!("could not connect the spatial output to the equalizer: {error}")
            });
        }
        thread::sleep(GRAPH_READY_INTERVAL);
    }
    Err("the spatial PipeWire graph did not become observable in time".into())
}

fn spatial_graph_exists() -> Result<bool, String> {
    Ok(spatial_graph_in(&pw_dump()?))
}

fn spatial_graph_in(objects: &[Value]) -> bool {
    objects.iter().any(|node| {
        matches!(
            node_name(node),
            Some(SPATIAL_SINK_NODE | SPATIAL_OUTPUT_NODE)
        )
    })
}

/// Reconcile intent with the observable controls. A command failure is not a
/// lifecycle failure: retaining the child is essential because it may carry
/// streams, and the next supervisor pass can retry after PipeWire recovers.
/// Returns `false` only when PipeWire was observed without the capture node,
/// i.e. the child is alive but no longer hosts the graph.
fn reconcile_mix_with_target<I: MixIntent>(target: impl FnOnce() -> Result<I, String>) -> bool {
    reconcile_mix_observed(None, target)
}

fn reconcile_mix_observed<I: MixIntent>(
    observed: Option<&[Value]>,
    target: impl FnOnce() -> Result<I, String>,
) -> bool {
    // Unlike the immediate CLI path, the supervisor can have spent a long
    // time in preflight since it chose SetMix. Re-read the persisted gate
    // inside the same lock that serializes ramps so it cannot restore an old
    // wet/dry target after a newer CLI toggle has completed.
    match set_mix_if_graph_exists_observed(observed, target) {
        Ok(true) => true,
        Ok(false) => {
            eprintln!("spatial supervisor: graph disappeared before its mix was updated");
            false
        }
        Err(error) => {
            eprintln!("spatial supervisor: could not update live mix: {error}");
            true
        }
    }
}

fn reconcile_mix_from_profile(
    load_profile: impl FnOnce() -> Result<spatial::SpatialProfile, String>,
) -> bool {
    reconcile_mix_with_target(load_profile)
}

fn reconcile_mix() -> bool {
    reconcile_mix_from_profile(spatial::load)
}

/// Consecutive supervisor passes a running graph may fail its readiness
/// check (a `pw-dump` timeout, the equalizer briefly restarting) before it is
/// torn down. A single transient failure must not move every stream.
const READINESS_GRACE_PASSES: u32 = 3;

/// Consecutive passes a live child may run without its capture node before
/// it is replaced.
const MISSING_GRAPH_PASSES: u32 = 3;

/// Whether a running graph should still be treated as ready although this
/// pass's readiness check failed transiently.
fn within_readiness_grace(child: ChildState, failed_passes: u32) -> bool {
    child == ChildState::Running && failed_passes <= READINESS_GRACE_PASSES
}

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

const SIGINT: c_int = 2;
const SIGTERM: c_int = 15;

unsafe extern "C" {
    fn signal(signum: c_int, handler: extern "C" fn(c_int)) -> usize;
}

extern "C" fn request_stop(_signal: c_int) {
    // Only an atomic store: async-signal-safe.
    STOP_REQUESTED.store(true, Ordering::SeqCst);
}

/// systemd stops the service with SIGTERM to this process only
/// (`KillMode=mixed`). The supervisor then drains the graph itself: an
/// `ExecStop` helper running beside it would race with the per-pass link
/// repair, and the equalizer router could promote the drained streams back
/// before the graph died.
fn install_stop_handler() {
    unsafe {
        signal(SIGTERM, request_stop);
        signal(SIGINT, request_stop);
    }
}

fn shutdown(child: Option<Child>) {
    let Some(mut active) = child else {
        return;
    };
    if let Err(error) = stop_graph(&mut active) {
        // Stopping regardless: the session manager re-routes what is left.
        eprintln!("spatial supervisor: {error}; stopping the graph anyway");
        let _ = active.kill();
        let _ = active.wait();
    }
}

/// Persistent systemd-service entrypoint. The graph is a child process owned
/// by this supervisor. Its lifecycle depends only on the explicit spatial
/// profile and a fresh, fail-closed readiness/target check.
pub fn run() -> Result<(), String> {
    install_stop_handler();
    let mut child: Option<Child> = None;
    let mut respawn_backoff = RespawnBackoff::default();
    let mut failed_readiness_passes = 0_u32;
    let mut missing_graph_passes = 0_u32;
    loop {
        if STOP_REQUESTED.load(Ordering::SeqCst) {
            shutdown(child);
            return Ok(());
        }
        let profile = spatial::load();
        // One `pw-dump` per pass, shared by every check below. Each dump costs
        // ~20 ms of CPU and ~450 KB of JSON; the previous two or three per
        // 500 ms pass kept the supervisor at 11-16% of a core while idle.
        let wants_graph = profile
            .as_ref()
            .is_ok_and(|profile| profile.mode == spatial::SpatialMode::BinauralStereo);
        let snapshot = (wants_graph || child.is_some()).then(pw_dump);
        let desired = match &snapshot {
            Some(Ok(objects)) => desired_config(objects),
            Some(Err(error)) => Err(error.clone()),
            None => desired_config(&[]),
        };
        let objects = snapshot.and_then(Result::ok);
        let mut steady = false;
        let mut ready = desired.as_ref().is_ok_and(|config| config.is_some());
        if desired.is_err() {
            failed_readiness_passes = failed_readiness_passes.saturating_add(1);
        } else {
            failed_readiness_passes = 0;
        }
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
        if let Err(error) = &desired
            && within_readiness_grace(child_state, failed_readiness_passes)
        {
            eprintln!(
                "spatial supervisor: readiness check failed ({failed_readiness_passes}/{READINESS_GRACE_PASSES}), keeping the graph: {error}"
            );
            ready = true;
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
                // `desired` is Ok only when this pass's snapshot succeeded.
                match objects
                    .as_deref()
                    .map(spatial_graph_in)
                    .ok_or("no PipeWire snapshot")
                {
                    Ok(true) => {
                        // An unowned graph: exiting lets systemd clean the
                        // cgroup instead of creating a second capture sink.
                        return Err(
                            "a spatial graph already exists; refusing to create a second graph"
                                .into(),
                        );
                    }
                    Ok(false) => {}
                    Err(error) => {
                        eprintln!("spatial supervisor: {error}");
                        thread::sleep(ROUTE_INTERVAL);
                        continue;
                    }
                }
                match write_config(&config).and_then(|path| spawn_connected_pipewire(&path)) {
                    Ok(spawned) => {
                        child = Some(spawned);
                        missing_graph_passes = 0;
                        respawn_backoff.record_spawn();
                        // The renderer already initializes the graph from its
                        // gate. Reconcile anyway in case intent changed while
                        // it spawned; a transient command failure never
                        // sacrifices the graph.
                        reconcile_mix();
                    }
                    Err(error) => {
                        // Treat a failed start like a crash so the backoff
                        // applies, instead of exiting and letting systemd
                        // recreate a client-visible sink every few seconds.
                        let delay = respawn_backoff.record_exit();
                        eprintln!(
                            "spatial supervisor: {error}; retrying in {} seconds",
                            delay.as_secs_f32()
                        );
                    }
                }
            }
            (SupervisorAction::SetMix, _, _) => {
                let link_result = match objects.as_deref() {
                    Some(objects) => repair_spatial_output_observed(objects),
                    None => Ok(false),
                };
                let links_intact = matches!(link_result, Ok(false));
                if let Some(objects) = objects.as_deref() {
                    upmix_spatial_streams(objects);
                }
                match link_result {
                    Ok(true) => {
                        eprintln!(
                            "spatial supervisor: reconnected the spatial output to the equalizer"
                        )
                    }
                    Ok(false) => {}
                    Err(error) => eprintln!(
                        "spatial supervisor: could not reconnect the spatial output: {error}"
                    ),
                }
                // This observes the live controls before ramping. It makes a
                // CLI transition idempotent for the supervisor rather than
                // replaying a second wet/dry ramp 500 ms later.
                if reconcile_mix_observed(objects.as_deref(), spatial::load) {
                    missing_graph_passes = 0;
                    steady = links_intact && failed_readiness_passes == 0;
                } else {
                    missing_graph_passes = missing_graph_passes.saturating_add(1);
                    if missing_graph_passes >= MISSING_GRAPH_PASSES
                        && let Some(mut stale) = child.take()
                    {
                        // The process is alive but hosts no graph (its module
                        // unloaded after a PipeWire restart). No client can be
                        // attached to a node that does not exist.
                        let _ = stale.kill();
                        let _ = stale.wait();
                        missing_graph_passes = 0;
                        let delay = respawn_backoff.record_exit();
                        eprintln!(
                            "spatial supervisor: replacing a graph process without nodes in {} seconds",
                            delay.as_secs_f32()
                        );
                    }
                }
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
        thread::sleep(if steady {
            STEADY_INTERVAL
        } else {
            ROUTE_INTERVAL
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::spatial::graph::{SPATIAL_DRY_CONTROLS, SPATIAL_WET_CONTROLS};
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
        // The fixtures expose the graph's built-in levels.
        spatial::SpatialProfile {
            schema: spatial::SCHEMA,
            mode,
            enabled,
            sound: spatial::SoundSettings::NEUTRAL,
            ..Default::default()
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
        // The output is unlinked from the equalizer before the drain so the
        // equalizer router cannot promote the moved streams back.
        runner.push_output("");
        runner.push_output("");
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
        let unlink = calls
            .iter()
            .position(|call| {
                call == &format!("pw-link --disconnect {SPATIAL_OUTPUT_FL} {EQUALIZER_INPUT_FL}")
            })
            .expect("the spatial output is unlinked before draining");
        let late_move = calls
            .iter()
            .position(|call| {
                call == "pactl move-sink-input 301 jambalinux-soniccore-game-equalizer"
            })
            .expect("late stream moved");
        assert!(unlink < late_move);
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
        runner.push_output("");
        runner.push_output("");
        for _ in 0..DRAIN_MAX_ATTEMPTS {
            runner.push_output(&inputs(&[(301, 400)]));
            runner.push_output(&inputs(&[(301, 400)]));
            queue_move(&runner, &[(301, 400)]);
        }
        // The kept graph is relinked to the proven equalizer input.
        runner.push_output(EQUALIZER_NODE);
        runner.push_output("");
        runner.push_output("");

        let _guard = crate::command::set_runner(runner.clone());
        crate::pipewire::install_mock_state_for_test();
        let mut child = live_child();
        let error = stop_graph(&mut child).expect_err("a non-empty sink must not be killed");

        assert!(error.contains("keeping the graph alive"));
        assert!(child.try_wait().expect("observe child").is_none());
        let calls = runner.command_lines();
        assert_registration_precedes_every_move(&calls);
        assert_eq!(
            calls.last().map(String::as_str),
            Some(format!("pw-link --wait {SPATIAL_OUTPUT_FR} {EQUALIZER_INPUT_FR}").as_str()),
            "a graph kept with streams must get its output back: {calls:?}"
        );
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

    fn linked_graph_dump(links: &[(&str, &str)]) -> String {
        // Nodes 87 (spatial output) and 74 (equalizer) with their stereo
        // ports, plus the requested links between them.
        let mut objects = vec![
            serde_json::json!({ "id": 74, "type": "PipeWire:Interface:Node", "info": { "props": {
                "node.name": TARGET_EQUALIZER_SINK, "media.class": "Audio/Sink" } } }),
            serde_json::json!({ "id": 88, "type": "PipeWire:Interface:Node", "info": { "props": {
                "node.name": SPATIAL_OUTPUT_NODE, "media.class": "Audio/Source" } } }),
        ];
        let ports = [
            (10, 88, "output_FL"),
            (11, 88, "output_FR"),
            (20, 74, "playback_FL"),
            (21, 74, "playback_FR"),
        ];
        for (id, node, name) in ports {
            objects.push(
                serde_json::json!({ "id": id, "type": "PipeWire:Interface:Port",
                "info": { "props": { "node.id": node, "port.name": name } } }),
            );
        }
        for (index, (output, input)) in links.iter().enumerate() {
            let port = |name: &str| ports.iter().find(|port| port.2 == name).unwrap().0;
            objects.push(
                serde_json::json!({ "id": 500 + index, "type": "PipeWire:Interface:Link",
                "info": { "output-port-id": port(output), "input-port-id": port(input) } }),
            );
        }
        serde_json::Value::Array(objects).to_string()
    }

    #[test]
    fn a_recreated_equalizer_gets_the_spatial_output_relinked() {
        let both = linked_graph_dump(&[("output_FL", "playback_FL"), ("output_FR", "playback_FR")]);
        let objects: Vec<Value> = serde_json::from_str(&both).unwrap();
        assert!(missing_output_links(&objects).is_empty());

        // The equalizer node was recreated: only the FL link survived.
        let partial = linked_graph_dump(&[("output_FL", "playback_FL")]);
        let objects: Vec<Value> = serde_json::from_str(&partial).unwrap();
        assert_eq!(
            missing_output_links(&objects),
            [(SPATIAL_OUTPUT_FR, EQUALIZER_INPUT_FR)]
        );

        let runner = Arc::new(crate::command::ScriptedRunner::new());
        runner.push_output(&linked_graph_dump(&[]));
        runner.push_output("");
        runner.push_output("");
        let _guard = crate::command::set_runner(runner.clone());
        assert!(repair_spatial_output().unwrap());
        assert_eq!(
            runner.command_lines()[1..],
            [
                format!("pw-link --wait {SPATIAL_OUTPUT_FL} {EQUALIZER_INPUT_FL}"),
                format!("pw-link --wait {SPATIAL_OUTPUT_FR} {EQUALIZER_INPUT_FR}"),
            ]
        );
    }

    #[test]
    fn repair_does_not_link_while_the_equalizer_ports_are_absent() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        runner.push_output(&spatial_capture_dump(1.0, 0.0));
        let _guard = crate::command::set_runner(runner.clone());
        assert!(!repair_spatial_output().unwrap());
        assert_eq!(runner.command_lines(), ["pw-dump"]);
    }

    #[test]
    fn a_transient_readiness_failure_does_not_tear_down_a_running_graph() {
        for pass in 1..=READINESS_GRACE_PASSES {
            assert!(within_readiness_grace(ChildState::Running, pass));
        }
        assert!(!within_readiness_grace(
            ChildState::Running,
            READINESS_GRACE_PASSES + 1
        ));
        assert!(!within_readiness_grace(ChildState::Absent, 1));
        assert!(!within_readiness_grace(ChildState::Exited, 1));
    }

    #[test]
    fn a_graph_without_nodes_is_stopped_when_its_pulse_sink_vanished() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // PulseAudio no longer lists the spatial sink, and PipeWire confirms
        // the capture node is gone too: no client can be attached.
        runner.push_output(&SINKS.replace("jambalinux-soniccore-game-spatial", "gone"));
        runner.push_output(EQUALIZER_NODE);
        let _guard = crate::command::set_runner(runner.clone());
        let mut child = live_child();
        stop_graph(&mut child).expect("an empty process may be stopped");
        assert!(child.try_wait().expect("observe child").is_some());
    }

    #[test]
    fn a_graph_hidden_only_from_pulse_is_kept() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // pipewire-pulse lost the sink but PipeWire still has the node, so
        // streams may still be attached: keep the graph.
        runner.push_output(&SINKS.replace("jambalinux-soniccore-game-spatial", "gone"));
        runner.push_output(&spatial_capture_dump(1.0, 0.0));
        let _guard = crate::command::set_runner(runner.clone());
        let mut child = live_child();
        let error = stop_graph(&mut child).expect_err("streams may still be attached");
        assert!(error.contains("keeping the graph alive"));
        assert!(child.try_wait().expect("observe child").is_none());
        reap(&mut child);
    }

    #[test]
    fn a_matching_snapshot_reconciles_the_mix_without_another_pw_dump() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        let _guard = crate::command::set_runner(runner.clone());
        let snapshot: Vec<Value> = serde_json::from_str(&spatial_capture_dump(1.0, 0.0)).unwrap();

        // Steady state: the snapshot already shows the wet target.
        assert!(reconcile_mix_observed(Some(&snapshot), || Ok(true)));
        assert!(
            runner.command_lines().is_empty(),
            "no extra command expected"
        );

        // The snapshot shows no capture node: reported as a missing graph.
        assert!(!reconcile_mix_observed(Some(&[]), || Ok(true)));
        assert!(runner.command_lines().is_empty());
    }

    #[test]
    fn a_mismatching_snapshot_is_reread_fresh_before_ramping() {
        let runner = Arc::new(crate::command::ScriptedRunner::new());
        // The fresh read under the lock already shows dry: a CLI toggle won
        // the race, so no ramp is sent from the stale snapshot.
        runner.push_output(&spatial_capture_dump(0.0, 1.0));
        let _guard = crate::command::set_runner(runner.clone());
        let stale: Vec<Value> = serde_json::from_str(&spatial_capture_dump(1.0, 0.0)).unwrap();

        assert!(reconcile_mix_observed(Some(&stale), || Ok(false)));
        assert_eq!(runner.command_lines(), ["pw-dump"]);
    }

    #[test]
    fn desired_config_checks_the_equalizer_in_the_shared_snapshot() {
        let objects: Vec<Value> = serde_json::from_str(EQUALIZER_NODE).unwrap();
        assert!(audio_sink_named(&objects, TARGET_EQUALIZER_SINK).is_ok());
        assert!(audio_sink_named(&[], TARGET_EQUALIZER_SINK).is_err());
        assert!(!spatial_graph_in(&objects));
        let graph: Vec<Value> = serde_json::from_str(&spatial_capture_dump(1.0, 0.0)).unwrap();
        assert!(spatial_graph_in(&graph));
    }

    #[test]
    fn only_spatial_streams_without_psd_upmix_are_updated() {
        let objects: Vec<Value> = serde_json::from_str(r#"[
            { "id": 50, "type": "PipeWire:Interface:Node", "info": { "props": {
                "node.name": "jambalinux-soniccore-game-spatial", "media.class": "Audio/Sink" } } },
            { "id": 60, "type": "PipeWire:Interface:Node", "info": { "props": {
                "media.class": "Stream/Output/Audio" },
                "params": { "Props": [ { "params": [ "channelmix.upmix-method", "none" ] } ] } } },
            { "id": 61, "type": "PipeWire:Interface:Node", "info": { "props": {
                "media.class": "Stream/Output/Audio" },
                "params": { "Props": [ { "params": [ "channelmix.upmix-method", "psd" ] } ] } } },
            { "id": 62, "type": "PipeWire:Interface:Node", "info": { "props": {
                "media.class": "Stream/Output/Audio" },
                "params": { "Props": [ { "params": [ "channelmix.upmix-method", "none" ] } ] } } },
            { "id": 70, "type": "PipeWire:Interface:Link", "info": { "output-node-id": 60, "input-node-id": 50 } },
            { "id": 71, "type": "PipeWire:Interface:Link", "info": { "output-node-id": 61, "input-node-id": 50 } },
            { "id": 72, "type": "PipeWire:Interface:Link", "info": { "output-node-id": 62, "input-node-id": 99 } }
        ]"#).unwrap();
        // 61 is already PSD; 62 plays elsewhere.
        assert_eq!(streams_needing_upmix(&objects), vec![60]);

        let runner = Arc::new(crate::command::ScriptedRunner::new());
        runner.push_output("");
        let _guard = crate::command::set_runner(runner.clone());
        upmix_spatial_streams(&objects);
        let calls = runner.command_lines();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("pw-cli set-param 60 Props"));
        assert!(calls[0].contains("psd"));
        assert!(streams_needing_upmix(&[]).is_empty());
    }

    #[test]
    fn small_changes_skip_the_ramp_and_large_ones_keep_it() {
        let game = crate::spatial::ListeningMode::Game.settings().unwrap();
        let mut nudged = game;
        nudged.bass_db += 0.5;
        let from = SpatialMix::for_sound(true, &game);
        assert_eq!(ramp_steps(from, SpatialMix::for_sound(true, &nudged)), 1);
        // Switching the 3D off moves the wet path a lot.
        assert_eq!(ramp_steps(from, SpatialMix::for_sound(false, &game)), MIX_RAMP_STEPS);
        let mut lifted = game;
        lifted.bass_db += 6.0;
        assert_eq!(ramp_steps(from, SpatialMix::for_sound(true, &lifted)), MIX_RAMP_STEPS);
    }
}
