//! Contract tests for `tools/diagnose-audio-transition.sh`: it must stay
//! read-only, fail clearly without its PipeWire/PulseAudio clients, and run
//! both modes against stub clients without touching the real audio graph.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/diagnose-audio-transition.sh")
}

fn source() -> String {
    fs::read_to_string(script()).expect("diagnostic script is readable")
}

fn audio_gain_contract() -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/protocol/diagnostics/audio-gain.md"),
    )
    .expect("audio gain evidence contract is readable")
}

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("soniccore-diag-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("scratch directory");
    directory
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("stub written");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("stub mode");
}

/// Stub clients that log every invocation. `busctl` is stubbed too so the
/// test never reaches the developer's real session bus.
fn stub_bin(root: &Path) -> (PathBuf, PathBuf) {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).expect("stub bin");
    let calls = root.join("calls.log");
    write_executable(
        &bin.join("pactl"),
        r#"#!/bin/sh
printf 'pactl %s\n' "$*" >> "$STUB_LOG"
case "$1" in
subscribe) echo "Event 'change' on sink #3"; exec sleep 30 ;;
get-default-sink) echo stub_sink ;;
info) echo 'Server Name: stub' ;;
list)
    case "$2" in
    short) printf '3\tstub_sink\tmodule\ts16le 2ch 48000Hz\tRUNNING\n' ;;
    sink-inputs) printf 'Sink Input #7\n\tSink: 3\n\tCorked: no\n\tProperties:\n\t\tapplication.name = "Stub"\n\t\tmedia.name = "Tone"\n' ;;
    *) echo '[]' ;;
    esac ;;
esac
"#,
    );
    write_executable(
        &bin.join("pw-dump"),
        r#"#!/bin/sh
printf 'pw-dump %s\n' "$*" >> "$STUB_LOG"
echo '[{"id":42,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"media.class":"Audio/Sink","node.name":"stub_sink","object.serial":99}}}]'
"#,
    );
    write_executable(
        &bin.join("busctl"),
        r#"#!/bin/sh
printf 'busctl %s\n' "$*" >> "$STUB_LOG"
"#,
    );
    write_executable(
        &bin.join("soniccore"),
        r#"#!/bin/sh
printf 'soniccore %s\n' "$*" >> "$STUB_LOG"
echo '{}'
"#,
    );
    (bin, calls)
}

fn run(args: &[&str], path: &str, stub_log: Option<&Path>) -> Output {
    // An absolute interpreter keeps the lookup independent of the test PATH.
    let bash = ["/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .find(|candidate| Path::new(candidate).exists())
        .expect("bash is installed");
    let mut command = Command::new(bash);
    command.arg(script()).args(args).env("PATH", path);
    if let Some(log) = stub_log {
        // Keep the spatial.json snapshot away from the developer's profile.
        command
            .env("STUB_LOG", log)
            .env("XDG_CONFIG_HOME", log.parent().expect("scratch directory"));
    }
    command.output().expect("bash runs the diagnostic script")
}

fn system_path(bin: &Path) -> String {
    format!("{}:/usr/bin:/bin", bin.display())
}

#[test]
fn script_is_executable_and_strict() {
    let mode = fs::metadata(script())
        .expect("script exists")
        .permissions()
        .mode();
    assert_ne!(mode & 0o111, 0, "the diagnostic script must be executable");
    assert!(source().contains("set -euo pipefail"));
}

#[test]
fn audio_gain_contract_keeps_measurement_passive_and_falsifiable() {
    let contract = audio_gain_contract();
    for required in [
        "Stream da aplicação",
        "Mixers seco/wet",
        "Convolvers/HRIR",
        "Quantum Game e Chat",
        "Volume percebido",
        "RMS em dBFS",
        "pico em dBFS",
        "Correlação normalizada",
        "formatos efetivamente negociados",
        "normalização",
        "upmix estéreo->7.1",
        "Game/Chat",
        "resampling",
        "Props",
        "VID/PID",
        "allowlists",
    ] {
        assert!(contract.contains(required), "contract omits `{required}`");
    }
    for forbidden in [
        "pactl move-sink-input",
        "pactl set-sink-volume",
        "pw-cli set-param",
        "qualquer escrita HID",
    ] {
        assert!(
            contract.contains(forbidden),
            "contract must prohibit `{forbidden}`"
        );
    }
}

#[test]
fn script_never_names_mutating_commands() {
    let text = source();
    for forbidden in [
        "move-sink-input",
        "set-default-sink",
        "set-param",
        "pw-link",
        "systemctl",
        "soniccore spatial enable",
        "soniccore spatial disable",
        "\"$soniccore\" spatial enable",
        "\"$soniccore\" spatial disable",
        "equalizer set",
        "equalizer enable",
        "equalizer disable",
        "equalizer preset",
        "equalizer profile",
    ] {
        assert!(
            !text.contains(forbidden),
            "the read-only diagnostic script mentions `{forbidden}`"
        );
    }
}

#[test]
fn help_describes_both_modes() {
    let output = run(&["--help"], "/usr/bin:/bin", None);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("spatial"));
    assert!(stdout.contains("timing"));
    assert!(stdout.contains("--output"));
}

#[test]
fn missing_clients_fail_clearly() {
    let root = scratch("missing");
    let empty = root.join("empty-bin");
    fs::create_dir_all(&empty).unwrap();
    let log = root.join("out.log");
    let spatial_args = ["spatial", "--output", log.to_str().unwrap()];
    for (mode, args) in [("spatial", &spatial_args[..]), ("timing", &["timing"][..])] {
        let output = run(args, empty.to_str().unwrap(), None);
        assert!(!output.status.success(), "{mode} must fail without clients");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("required command(s) not found"), "{stderr}");
        assert!(
            stderr.contains("pactl") && stderr.contains("pw-dump"),
            "{stderr}"
        );
        assert!(
            !stderr.contains("line "),
            "no shell trace expected: {stderr}"
        );
    }
    let _ = fs::remove_dir_all(root);
}

fn assert_only_read_only_calls(calls: &Path) {
    let log = fs::read_to_string(calls).unwrap_or_default();
    for line in log.lines() {
        let allowed = [
            "pactl subscribe",
            "pactl get-default-sink",
            "pactl info",
            "pactl list",
            "pw-dump",
            "busctl --user",
            "soniccore equalizer status --format json",
            "soniccore spatial status --format json",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix));
        assert!(allowed, "unexpected client call: {line}");
    }
}

#[test]
fn timing_reports_status_commands() {
    let root = scratch("timing");
    let (bin, calls) = stub_bin(&root);
    let soniccore = bin.join("soniccore");
    let output = run(
        &[
            "timing",
            "--runs",
            "2",
            "--soniccore",
            soniccore.to_str().unwrap(),
        ],
        &system_path(&bin),
        Some(&calls),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("soniccore equalizer status --format json"));
    assert!(stdout.contains("soniccore spatial status --format json"));
    assert!(stdout.contains("median_ms"));
    assert_only_read_only_calls(&calls);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn spatial_records_snapshots_and_refuses_overwrite() {
    let root = scratch("spatial");
    let (bin, calls) = stub_bin(&root);
    let log = root.join("transition.log");
    let path = system_path(&bin);
    let output = run(
        &[
            "spatial",
            "--duration",
            "1",
            "--interval",
            "0.2",
            "--output",
            log.to_str().unwrap(),
        ],
        &path,
        Some(&calls),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let recorded = fs::read_to_string(&log).expect("log written");
    assert!(recorded.contains("default   stub_sink"), "{recorded}");
    assert!(
        recorded.contains("input=7 sink=3 corked=no app=Stub media=Tone"),
        "{recorded}"
    );
    assert!(
        recorded.contains("pulse     Event 'change' on sink #3"),
        "{recorded}"
    );
    assert!(recorded.contains("recording finished"), "{recorded}");
    if ["/usr/bin/python3", "/bin/python3"]
        .iter()
        .any(|candidate| Path::new(candidate).exists())
    {
        assert!(recorded.contains("name=stub_sink"), "{recorded}");
    }
    // No temporary FIFO directory is left next to the log.
    let leftovers = fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".diagnose"))
        .count();
    assert_eq!(leftovers, 0);
    assert_only_read_only_calls(&calls);

    let again = run(
        &[
            "spatial",
            "--duration",
            "1",
            "--output",
            log.to_str().unwrap(),
        ],
        &path,
        Some(&calls),
    );
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("refusing to overwrite"));
    let _ = fs::remove_dir_all(root);
}
