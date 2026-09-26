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
use std::time::Duration;

use serde_json::Value;

use crate::spatial;
use crate::spatial::graph::{SPATIAL_OUTPUT_NODE, SPATIAL_SINK_NODE, TARGET_EQUALIZER_SINK};

const CONFIG_NAME: &str = "pipewire-spatial.conf";
const ROUTE_INTERVAL: Duration = Duration::from_millis(500);
const GRAPH_READY_ATTEMPTS: usize = 40;
const GRAPH_READY_INTERVAL: Duration = Duration::from_millis(50);

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
    if !profile.enabled || profile.mode != spatial::SpatialMode::BinauralStereo {
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
    spatial::graph::render_filter_chain_config(Path::new(&hrir)).map(Some)
}

fn spawn_pipewire(config: &Path) -> Result<Child, String> {
    Command::new("pipewire")
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

/// Persistent systemd-service entrypoint. The graph is a child process owned
/// by this supervisor. Its lifecycle depends only on the explicit spatial
/// profile and a fresh, fail-closed readiness/target check.
pub fn run() -> Result<(), String> {
    let mut child: Option<Child> = None;
    loop {
        let desired = desired_config();
        match (child.as_mut(), desired) {
            (None, Ok(Some(config))) => {
                if spatial_graph_exists()? {
                    return Err(
                        "a spatial graph already exists; refusing to create a second graph".into(),
                    );
                }
                child = Some(spawn_connected_pipewire(&write_config(&config)?)?);
            }
            (Some(active), Ok(None)) => {
                // A drain failure is deliberately non-fatal. Dropping the
                // child here would orphan a graph that can still carry client
                // streams; retain it and retry from a fresh PulseAudio view.
                if let Err(error) = stop_graph(active) {
                    eprintln!("spatial supervisor: {error}");
                } else {
                    child = None;
                }
            }
            (Some(active), Ok(Some(_))) => {
                if let Some(status) = active.try_wait().map_err(|error| {
                    format!("could not monitor the spatial PipeWire graph: {error}")
                })? {
                    return Err(format!("spatial PipeWire graph exited with {status}"));
                }
            }
            (None, Ok(None)) => {}
            (Some(active), Err(error)) => {
                // Do not remove a live graph unless its inputs can first be
                // restored to the proven equalizer target.
                if let Err(restore) = restore_streams() {
                    eprintln!(
                        "spatial supervisor: preflight failed: {error}; retaining the live graph because stream restoration is unsafe: {restore}"
                    );
                } else {
                    stop_graph(active)?;
                    child = None;
                }
            }
            (None, Err(error)) => eprintln!("spatial supervisor: {error}"),
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
    fn teardown_moves_a_late_unregistered_stream_before_confirming_empty_and_killing() {
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
}
