use std::fs;
use std::process::Command;

#[test]
fn widget_node_tests_are_part_of_cargo_validation() {
    let mut tests = fs::read_dir("tests")
        .expect("the repository tests directory must be readable")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("widget-") && name.ends_with(".test.mjs"))
        })
        .collect::<Vec<_>>();
    tests.sort();
    assert!(
        !tests.is_empty(),
        "expected at least one tests/widget-*.test.mjs file"
    );

    let output = Command::new("node")
        .arg("--test")
        .args(&tests)
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "Node.js is required to run widget tests (node --test tests/widget-*.test.mjs): {error}"
            )
        });

    assert!(
        output.status.success(),
        "widget Node tests failed (status {}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
