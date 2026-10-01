//! Runs the Claude Pro MCP bridge's Python unit tests under `cargo test`.

use std::process::Command;

#[test]
fn claude_pro_mcp_python_tests_pass() {
    let output = Command::new("python3")
        .args(["-m", "unittest", "tests/test_claude_pro_mcp.py"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("python3 is required to run the Claude Pro MCP bridge tests");

    assert!(
        output.status.success(),
        "Claude Pro MCP bridge tests failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
