//! Contract tests for the strictly observational audio-gain collector.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static SCRATCH_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn script() -> PathBuf {
    root().join("tools/diagnose-audio-gain.sh")
}
fn scratch() -> PathBuf {
    let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "soniccore-audio-gain-{}-{sequence}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("bin")).unwrap();
    path
}
fn executable(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new("/bin/bash")
        .arg(script())
        .args(args)
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", root.join("bin").display()),
        )
        .env("STUB_LOG", root.join("calls.log"))
        .output()
        .unwrap()
}

#[test]
fn source_has_no_mutating_graph_or_soniccore_command() {
    let text = fs::read_to_string(script()).unwrap();
    for forbidden in [
        "move-sink-input",
        "set-sink-volume",
        "set-sink-input-volume",
        "set-default-sink",
        "set-param",
        "pw-link",
        "systemctl",
        "set-volume",
        "spatial enable",
        "spatial disable",
        "equalizer set",
        "equalizer reset",
    ] {
        assert!(!text.contains(forbidden), "forbidden token: {forbidden}");
    }
}

#[test]
fn simulated_clients_prove_queries_and_log_all_evidence() {
    let tmp = scratch();
    let bin = tmp.join("bin");
    executable(
        &bin.join("pactl"),
        "#!/bin/sh\nprintf 'pactl %s\\n' \"$*\" >> \"$STUB_LOG\"\ncase \"$*\" in *sink-inputs*) printf 'Sink Input #4\\n\\tVolume: front-left: 65536 / 100%% / 0.00 dB\\n\\tMute: no\\n\\tProperties:\\n\\t\\tapplication.name = \"Sensitive Player\"\\n';; *sinks*) printf 'Sink #2\\n\\tName: Quantum Game\\n\\tVolume: front-left: 65536 / 100%% / 0.00 dB\\n\\tMute: no\\n\\tSample Specification: float32le 2ch 48000Hz\\n';; *) echo Quantum_Game;; esac\n",
    );
    executable(
        &bin.join("pw-dump"),
        "#!/bin/sh\nprintf 'pw-dump %s\\n' \"$*\" >> \"$STUB_LOG\"\necho '{\"node.name\":\"spatial\",\"audio.position\":[\"FL\",\"FR\"],\"audio.rate\":48000,\"Params\":\"Props\",\"wetDryL:Gain 1\":1.0,\"link.output.node\":1}'\n",
    );
    executable(
        &bin.join("pw-metadata"),
        "#!/bin/sh\nprintf 'pw-metadata %s\\n' \"$*\" >> \"$STUB_LOG\"\necho metadata\n",
    );
    executable(
        &bin.join("soniccore"),
        "#!/bin/sh\nprintf 'soniccore %s\\n' \"$*\" >> \"$STUB_LOG\"\necho '{\"schema\":1}'\n",
    );
    let output = tmp.join("gain.log");
    let result = run(
        &tmp,
        &[
            "--duration",
            "1",
            "--interval",
            "0.1",
            "--output",
            output.to_str().unwrap(),
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let record = fs::read_to_string(&output).unwrap();
    for required in [
        "monotonic_us=",
        "pactl list sinks",
        "Sensitive Player",
        "pw-dump",
        "audio.position",
        "wetDryL:Gain 1",
        "pw-metadata",
        "soniccore status --format json",
        "soniccore equalizer status --format json",
        "soniccore spatial status --format json",
        "hrir-file-metadata",
    ] {
        assert!(record.contains(required), "missing {required}");
    }
    let calls = fs::read_to_string(tmp.join("calls.log")).unwrap();
    for call in calls.lines() {
        assert!(
            call.starts_with("pactl get-default-sink")
                || call.starts_with("pactl list sinks")
                || call.starts_with("pactl list sink-inputs")
                || call.starts_with("pw-dump")
                || call.starts_with("pw-metadata")
                || call == "soniccore status --format json"
                || call == "soniccore equalizer status --format json"
                || call == "soniccore spatial status --format json",
            "unexpected call: {call}"
        );
    }
    let again = run(&tmp, &["--output", output.to_str().unwrap()]);
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("refusing to overwrite"));
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn missing_required_tools_are_explicit() {
    let tmp = scratch();
    let output = tmp.join("gain.log");
    let result = Command::new("/bin/bash")
        .arg(script())
        .arg("--output")
        .arg(output)
        .env("PATH", tmp.join("bin"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("required command not found in PATH: pactl"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let _ = fs::remove_dir_all(tmp);
}
