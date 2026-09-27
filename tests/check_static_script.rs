//! Runs the static-check script's success, optional-tool, and failure paths
//! under `cargo test`.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static SCRATCH_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

fn bash() -> &'static str {
    ["/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .find(|candidate| Path::new(candidate).exists())
        .expect("bash is installed")
}

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/check-static.sh")
}

fn run(root: Option<&Path>, path: Option<&Path>) -> Output {
    let mut command = Command::new(bash());
    command
        .arg(script())
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env_remove("CHECK_STATIC_ROOT");
    if let Some(root) = root {
        command.env("CHECK_STATIC_ROOT", root);
    }
    if let Some(path) = path {
        command.env("PATH", path);
    }
    command.output().expect("bash runs the static-check script")
}

fn scratch(name: &str) -> PathBuf {
    let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "soniccore-check-static-{}-{sequence}-{name}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("scratch directory");
    directory
}

fn output_message(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

#[test]
fn check_static_script_exercises_success_optional_tool_and_failure_paths() {
    let normal = run(None, None);
    assert!(
        normal.status.success(),
        "the repository static checks must pass:\n{}",
        output_message(&normal)
    );

    // Restrict PATH to the interpreter and the one external utility used by
    // the script. This proves absent optional linters produce warnings rather
    // than causing an interactive prompt or a failure.
    let reduced_root = scratch("reduced-path");
    let reduced_bin = reduced_root.join("bin");
    fs::create_dir_all(&reduced_bin).expect("reduced bin directory");
    symlink(bash(), reduced_bin.join("bash")).expect("bash symlink");
    let dirname = ["/bin/dirname", "/usr/bin/dirname"]
        .into_iter()
        .find(|candidate| Path::new(candidate).exists())
        .expect("dirname is installed");
    symlink(dirname, reduced_bin.join("dirname")).expect("dirname symlink");
    let reduced = run(None, Some(&reduced_bin));
    assert!(
        reduced.status.success(),
        "missing optional tools must not fail static checks:\n{}",
        output_message(&reduced)
    );
    assert!(
        String::from_utf8_lossy(&reduced.stderr).contains("skipping"),
        "missing optional tools must be reported:\n{}",
        output_message(&reduced)
    );

    // Point the same production script at a disposable project with invalid
    // shell syntax. `set -e` must carry bash -n's failure to the caller.
    let broken_root = scratch("broken-script");
    let broken_tools = broken_root.join("tools");
    fs::create_dir_all(&broken_tools).expect("broken tools directory");
    fs::write(broken_tools.join("broken.sh"), "if then\n").expect("broken script written");
    let broken = run(Some(&broken_root), None);
    assert!(
        !broken.status.success(),
        "a real syntax error must fail static checks:\n{}",
        output_message(&broken)
    );

    fs::remove_dir_all(reduced_root).expect("reduced scratch cleanup");
    fs::remove_dir_all(broken_root).expect("broken scratch cleanup");
}
