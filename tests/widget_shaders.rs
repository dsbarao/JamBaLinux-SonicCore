//! The widget ships compiled shader packs (.qsb). They must match their GLSL
//! sources: recompile each one and compare bytes (qsb output is deterministic).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn qsb() -> Option<PathBuf> {
    ["qsb6", "qsb"]
        .iter()
        .find_map(|name| {
            Command::new("sh")
                .args(["-c", &format!("command -v {name}")])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
        })
        .or_else(|| Some(PathBuf::from("/usr/lib/qt6/bin/qsb")).filter(|path| path.exists()))
}

#[test]
fn compiled_shaders_match_their_sources() {
    let shaders = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("packaging/plasma/org.jambalinux.soniccore/contents/shaders");
    let qsb = qsb().expect("qsb (Qt Shader Tools) is required to check the widget shaders");
    let scratch = std::env::temp_dir().join(format!("soniccore-qsb-{}", std::process::id()));
    fs::create_dir_all(&scratch).unwrap();

    let mut checked = 0;
    for entry in fs::read_dir(&shaders).unwrap() {
        let source = entry.unwrap().path();
        if source.extension().and_then(|extension| extension.to_str()) != Some("frag") {
            continue;
        }
        let name = source.file_name().unwrap().to_string_lossy().into_owned();
        let fresh = scratch.join(format!("{name}.qsb"));
        let output = Command::new(&qsb)
            .args([
                "--glsl",
                "100es,120,150",
                "--hlsl",
                "50",
                "--msl",
                "12",
                "-o",
            ])
            .arg(&fresh)
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name} does not compile:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let shipped = fs::read(shaders.join(format!("{name}.qsb")))
            .unwrap_or_else(|_| panic!("{name}.qsb is missing; run tools/build-shaders.sh"));
        assert_eq!(
            fs::read(&fresh).unwrap(),
            shipped,
            "{name}.qsb is out of date; run tools/build-shaders.sh"
        );
        checked += 1;
    }
    assert!(checked >= 2, "expected the particles and waves shaders");
    let _ = fs::remove_dir_all(scratch);
}
