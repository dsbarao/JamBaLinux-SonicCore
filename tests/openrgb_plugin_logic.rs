//! Builds and runs the OpenRGB plugin's logic tests (tests/openrgb-plugin.test.cpp)
//! with g++ and Qt6Core. OpenRGB itself is not needed for these tests.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn openrgb_plugin_logic_tests_pass() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let scratch = std::env::temp_dir().join(format!(
        "jambalinux-openrgb-plugin-test-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();

    let qt = Command::new("pkg-config")
        .args(["--cflags", "--libs", "Qt6Core"])
        .output()
        .expect("pkg-config is required to build the OpenRGB plugin tests");
    assert!(
        qt.status.success(),
        "Qt6Core development files are required"
    );
    let qt_flags = String::from_utf8(qt.stdout).unwrap();

    let binary = scratch.join("openrgb-plugin-test");
    let build = Command::new("g++")
        .args(["-std=c++17", "-Wall", "-Wextra", "-Werror", "-fPIC", "-o"])
        .arg(&binary)
        .arg(root.join("tests/openrgb-plugin.test.cpp"))
        .arg(root.join("openrgb-plugin/SoniccoreClient.cpp"))
        .args(qt_flags.split_whitespace())
        .arg("-pthread")
        .output()
        .expect("g++ is required to build the OpenRGB plugin tests");
    assert!(
        build.status.success(),
        "plugin test build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&binary)
        .arg(&scratch)
        .current_dir(&scratch)
        // The writer publishes the requested colors there; never the
        // session's real runtime directory.
        .env("XDG_RUNTIME_DIR", &scratch)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "plugin logic tests failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let _ = fs::remove_dir_all(&scratch);
}
