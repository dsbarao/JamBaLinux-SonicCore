//! Integration checks for the offline, supplied-file-only gain ledger.

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn fixture(name: &str) -> PathBuf {
    root().join("tests/fixtures/audio_gain").join(name)
}

fn analyze(name: &str) -> serde_json::Value {
    let output = Command::new("python3")
        .arg(root().join("tools/analyze_audio_gain.py"))
        .arg(fixture(name))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn game_bypass_reports_observed_values_and_dry_budget() {
    let report = analyze("game-bypass.json");
    assert_eq!(
        report["observed"]["wet_dry_state"],
        "bypass nominal (wet=0, dry=1)"
    );
    assert_eq!(
        report["limits_derived"]["dry_downmix_maximum_linear"],
        3.121
    );
    assert_eq!(
        report["comparisons"]["game_vs_chat"]["format_or_rate_divergent"],
        false
    );
}

#[test]
fn chat_binaural_and_divergent_endpoints_remain_measurement_not_gain_claims() {
    let report = analyze("chat-binaural.json");
    assert_eq!(
        report["observed"]["wet_dry_state"],
        "binaural nominal (wet=1, dry=0)"
    );
    assert_eq!(
        report["comparisons"]["game_vs_chat"]["format_or_rate_divergent"],
        true
    );
    assert!(
        report["indeterminate"]["endpoint_gain_or_spl"]
            .as_str()
            .unwrap()
            .contains("indeterminado")
    );
}

#[test]
fn positive_eq_and_measured_overage_raise_clipping_risks() {
    let report = analyze("positive-eq.json");
    let risks = report["clipping_risks"].as_array().unwrap();
    assert!(risks.iter().any(|risk| risk["kind"] == "positive-eq"));
    assert!(risks.iter().any(|risk| risk["kind"] == "measured-peak"));
    assert!(
        report["recommendation"]
            .as_str()
            .unwrap()
            .contains("não aumentar volume acima de 100%")
    );
}

#[test]
fn missing_fields_are_indeterminate_not_invented() {
    let report = analyze("missing-fields.json");
    assert!(
        report["observed"]["eq_applied_bands_db"]
            .as_str()
            .unwrap()
            .contains("indeterminado")
    );
    assert!(
        report["indeterminate"]["convolver_hrir_gain"]
            .as_str()
            .unwrap()
            .contains("indeterminado")
    );
}

#[test]
fn divergent_format_fixture_marks_transition_and_requires_no_audio_services() {
    let report = analyze("format-rate-divergent.json");
    assert!(
        report["observed"]["wet_dry_state"]
            .as_str()
            .unwrap()
            .contains("transição")
    );
    assert_eq!(
        report["comparisons"]["game_vs_chat"]["format_or_rate_divergent"],
        true
    );
    let source = std::fs::read_to_string(root().join("tools/analyze_audio_gain.py")).unwrap();
    for forbidden in [
        "pactl",
        "pw-",
        "systemctl",
        "soniccore",
        "subprocess",
        "os.system",
    ] {
        assert!(
            !source.contains(forbidden),
            "offline analyzer contains {forbidden}"
        );
    }
}
