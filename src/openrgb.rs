//! "Vincular com OpenRGB": reports whether the SonicCore plugin for OpenRGB
//! is installed and up to date, and installs it. The plugin's sources and its
//! install script are embedded in this binary, so the button works without a
//! copy of the repository; the build itself is tools/install-openrgb-plugin.sh
//! (downloads the matching OpenRGB headers, compiles, copies the .so).
//! It never edits OpenRGB's own settings and never restarts OpenRGB.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PLUGIN_FILE: &str = "libJamBaLinuxSonicCorePlugin.so";
/// The name OpenRGB lists the plugin under in its settings, and the name it
/// had before the rebranding (an entry OpenRGB may still keep).
const PLUGIN_NAME: &str = "JamBaLinux Sonic Core";
const OLD_PLUGIN_NAME: &str = "JamBaLinux SonicCore";
const SUPPORTED_VERSIONS: &[&str] = &["1.1"];
const BUILD_TOOLS: &[&str] = &["qmake6", "make", "g++", "git"];

/// Everything the build needs, at the paths the script expects.
const SOURCES: &[(&str, &str)] = &[
    (
        "tools/install-openrgb-plugin.sh",
        include_str!("../tools/install-openrgb-plugin.sh"),
    ),
    (
        "openrgb-plugin/JamBaLinuxOpenRGBPlugin.pro",
        include_str!("../openrgb-plugin/JamBaLinuxOpenRGBPlugin.pro"),
    ),
    (
        "openrgb-plugin/JamBaLinuxPlugin.cpp",
        include_str!("../openrgb-plugin/JamBaLinuxPlugin.cpp"),
    ),
    (
        "openrgb-plugin/JamBaLinuxPlugin.h",
        include_str!("../openrgb-plugin/JamBaLinuxPlugin.h"),
    ),
    (
        "openrgb-plugin/JamBaLinuxPlugin.json",
        include_str!("../openrgb-plugin/JamBaLinuxPlugin.json"),
    ),
    (
        "openrgb-plugin/LightingPlan.h",
        include_str!("../openrgb-plugin/LightingPlan.h"),
    ),
    (
        "openrgb-plugin/SoniccoreClient.cpp",
        include_str!("../openrgb-plugin/SoniccoreClient.cpp"),
    ),
    (
        "openrgb-plugin/SoniccoreClient.h",
        include_str!("../openrgb-plugin/SoniccoreClient.h"),
    ),
];

/// FNV-1a over the embedded sources: tells whether the installed plugin was
/// built from this version of SonicCore. Stable across Rust versions.
fn sources_fingerprint() -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (path, contents) in SOURCES {
        for byte in path.bytes().chain([0]).chain(contents.bytes()).chain([0]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".into())
}

fn config_home() -> Result<PathBuf, String> {
    match std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(home()?.join(".config")),
    }
}

fn state_home() -> Result<PathBuf, String> {
    match std::env::var_os("XDG_STATE_HOME").filter(|value| !value.is_empty()) {
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(home()?.join(".local/state")),
    }
}

/// Kept outside OpenRGB's plugin folder, which OpenRGB tries to load.
fn stamp_path() -> Result<PathBuf, String> {
    Ok(state_home()?.join("jambalinux-soniccore/openrgb-plugin.stamp"))
}

fn find_program(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// The version from `openrgb --version` ("  Version:\t\t 1.1").
pub fn parse_version(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Version:")
            .map(|version| version.trim().to_owned())
            .filter(|version| !version.is_empty())
    })
}

/// Whether OpenRGB's settings list the plugin as enabled; `None` when it
/// has never loaded it (OpenRGB adds the entry, enabled, on first load).
pub fn plugin_enabled(settings: &str) -> Option<bool> {
    let value: serde_json::Value = serde_json::from_str(settings).ok()?;
    let plugins = value["Plugins"]["plugins"].as_array()?;
    [PLUGIN_NAME, OLD_PLUGIN_NAME].into_iter().find_map(|name| {
        plugins
            .iter()
            .find(|plugin| plugin["name"].as_str() == Some(name))
            .and_then(|plugin| plugin["enabled"].as_bool())
    })
}

fn openrgb_running() -> bool {
    Command::new("pgrep")
        .args(["-x", "openrgb"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn status_json() -> Result<serde_json::Value, String> {
    let installed = find_program("openrgb").is_some();
    let version = if installed {
        Command::new("openrgb")
            .arg("--version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .and_then(|output| parse_version(&String::from_utf8_lossy(&output.stdout)))
    } else {
        None
    };
    let supported = version
        .as_deref()
        .is_some_and(|version| SUPPORTED_VERSIONS.contains(&version));
    let config = config_home()?.join("OpenRGB");
    let plugin_installed = config.join("plugins").join(PLUGIN_FILE).is_file();
    let current = plugin_installed.then(|| {
        fs::read_to_string(stamp_path().ok()?)
            .ok()
            .map(|stamp| stamp.trim() == sources_fingerprint())
    });
    let enabled = fs::read_to_string(config.join("OpenRGB.json"))
        .ok()
        .and_then(|settings| plugin_enabled(&settings));
    let missing_tools = BUILD_TOOLS
        .iter()
        .filter(|tool| find_program(tool).is_none())
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "schema": 1,
        "openrgb_installed": installed,
        "openrgb_version": version,
        "supported_versions": SUPPORTED_VERSIONS,
        "supported": supported,
        "openrgb_running": openrgb_running(),
        "plugin_installed": plugin_installed,
        // null: installed before SonicCore kept track of it.
        "plugin_current": current.flatten(),
        "plugin_enabled": enabled,
        "missing_tools": missing_tools,
    }))
}

fn write_sources(root: &Path) -> Result<(), String> {
    for (path, contents) in SOURCES {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        fs::write(&target, contents).map_err(|error| format!("{}: {error}", target.display()))?;
    }
    Ok(())
}

/// Builds and installs the plugin; returns the script's messages.
fn link() -> Result<String, String> {
    let status = status_json()?;
    if status["openrgb_installed"] != true {
        return Err("O OpenRGB não está instalado.".into());
    }
    if status["supported"] != true {
        return Err(format!(
            "OpenRGB {} não é suportado; versões suportadas: {}.",
            status["openrgb_version"].as_str().unwrap_or("?"),
            SUPPORTED_VERSIONS.join(", ")
        ));
    }
    let missing = status["missing_tools"].as_array().cloned().unwrap_or_default();
    if !missing.is_empty() {
        let names = missing.iter().filter_map(|tool| tool.as_str()).collect::<Vec<_>>();
        return Err(format!("Faltam ferramentas de compilação: {}.", names.join(", ")));
    }

    let root = std::env::temp_dir().join(format!("soniccore-openrgb-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let result = write_sources(&root).and_then(|()| {
        let output = Command::new("bash")
            .arg(root.join("tools/install-openrgb-plugin.sh"))
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("bash: {error}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if output.status.success() {
            // git's progress chatter goes to stderr; only failures need it.
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("{stdout}\n{}", stderr.trim()).trim().to_owned())
        }
    });
    let _ = fs::remove_dir_all(&root);
    let message = result?;

    let stamp = stamp_path()?;
    if let Some(parent) = stamp.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    fs::write(&stamp, sources_fingerprint())
        .map_err(|error| format!("{}: {error}", stamp.display()))?;
    Ok(message)
}

/// `soniccore openrgb [--format json]` and `soniccore openrgb link [--format json]`.
pub fn command(mut args: impl Iterator<Item = String>) -> Result<bool, String> {
    let first = args.next();
    let (verb, format) = match first.as_deref() {
        Some("link") => (Some("link"), args.next()),
        _ => (None, first),
    };
    let json = match (format.as_deref(), args.next().as_deref(), args.next()) {
        (None, _, _) => false,
        (Some("--format"), Some("json"), None) => true,
        _ => return Err("usage: soniccore openrgb [link] [--format json]".into()),
    };

    if verb == Some("link") {
        let result = link();
        if json {
            let (ok, message) = match &result {
                Ok(message) => (true, message.clone()),
                Err(message) => (false, message.clone()),
            };
            println!(
                "{}",
                serde_json::json!({
                    "schema": 1,
                    "ok": ok,
                    "message": message,
                    "openrgb_running": openrgb_running(),
                })
            );
            // The result is in the JSON; the exit code stays 0 so the
            // interface always reads it.
            return Ok(true);
        }
        let message = result?;
        println!("{message}");
        return Ok(true);
    }

    let status = status_json()?;
    if json {
        println!("{status}");
    } else {
        println!("{}", serde_json::to_string_pretty(&status).map_err(|error| error.to_string())?);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_line_is_read_from_openrgb_output() {
        let output = "OpenRGB 1.1, for controlling RGB lighting.\n  Version:\t\t 1.1\n  Build Date\t\t x\n";
        assert_eq!(parse_version(output).as_deref(), Some("1.1"));
        assert_eq!(parse_version("nothing here"), None);
    }

    #[test]
    fn the_plugin_entry_in_openrgb_settings_is_found() {
        let settings = r#"{"Plugins":{"plugins":[
            {"name":"OpenRGB Effects Plugin","enabled":true},
            {"name":"JamBaLinux SonicCore","enabled":false}]}}"#;
        assert_eq!(plugin_enabled(settings), Some(false));
        // The current name wins over an entry left from the old one.
        let both = r#"{"Plugins":{"plugins":[
            {"name":"JamBaLinux SonicCore","enabled":false},
            {"name":"JamBaLinux Sonic Core","enabled":true}]}}"#;
        assert_eq!(plugin_enabled(both), Some(true));
        assert_eq!(plugin_enabled(r#"{"Plugins":{"plugins":[]}}"#), None);
        assert_eq!(plugin_enabled("not json"), None);
    }

    #[test]
    fn embedded_sources_are_written_where_the_script_expects_them() {
        let root = std::env::temp_dir().join(format!("soniccore-openrgb-test-{}", std::process::id()));
        write_sources(&root).unwrap();
        assert!(root.join("tools/install-openrgb-plugin.sh").is_file());
        assert!(root.join("openrgb-plugin/JamBaLinuxOpenRGBPlugin.pro").is_file());
        // Every file the qmake project lists is embedded.
        let project = fs::read_to_string(root.join("openrgb-plugin/JamBaLinuxOpenRGBPlugin.pro")).unwrap();
        for name in ["JamBaLinuxPlugin.cpp", "SoniccoreClient.cpp", "JamBaLinuxPlugin.h", "SoniccoreClient.h", "LightingPlan.h"] {
            assert!(project.contains(name), "{name} is not in the .pro");
        }
        fs::remove_dir_all(root).unwrap();
        assert_eq!(sources_fingerprint(), sources_fingerprint());
        assert_eq!(sources_fingerprint().len(), 16);
    }
}
