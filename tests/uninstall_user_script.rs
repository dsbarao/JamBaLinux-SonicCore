//! Runs the shell-level uninstaller contract tests under `cargo test`.

use std::path::Path;
use std::process::Command;

#[test]
fn uninstall_user_script_contracts_pass() {
    // Use an absolute interpreter so the test does not depend on the test PATH.
    let bash = ["/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .find(|candidate| Path::new(candidate).exists())
        .expect("bash is installed");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/uninstall-user.test.sh");

    let output = Command::new(bash)
        .arg(script)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("bash runs the uninstaller contract tests");

    assert!(
        output.status.success(),
        "uninstaller contract tests failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("uninstall-user tests passed"),
        "uninstaller contract tests did not report completion"
    );
}
