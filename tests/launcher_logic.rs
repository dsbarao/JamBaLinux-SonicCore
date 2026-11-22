//! Builds and runs the app launcher's logic tests (tests/launcher) with
//! qmake from Qt 6. No window, media player or PipeWire is needed.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn qmake() -> Option<String> {
    ["qmake6", "qmake-qt6", "/usr/lib/qt6/bin/qmake", "/usr/lib64/qt6/bin/qmake"]
        .into_iter()
        .find(|candidate| {
            Command::new(candidate)
                .args(["-query", "QT_VERSION"])
                .output()
                .is_ok_and(|output| output.status.success() && output.stdout.starts_with(b"6."))
        })
        .map(str::to_owned)
}

#[test]
fn launcher_logic_tests_pass() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let scratch = std::env::temp_dir().join(format!("jambalinux-launcher-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let qmake = qmake().expect("qmake from Qt 6 is required to build the launcher tests");

    let configure = Command::new(&qmake)
        .arg(root.join("tests/launcher/launcher-test.pro"))
        .current_dir(&scratch)
        .output()
        .unwrap();
    assert!(configure.status.success(), "qmake failed:\n{}", String::from_utf8_lossy(&configure.stderr));
    let build = Command::new("make")
        .arg("-j4")
        .current_dir(&scratch)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "launcher test build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(scratch.join("launcher-test")).output().unwrap();
    assert!(
        run.status.success(),
        "launcher logic tests failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let _ = fs::remove_dir_all(&scratch);
}
