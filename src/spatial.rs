//! Experimental open spatial/binaural audio: configuration, state, and gate.
//!
//! This module is intentionally a foundation only. It stores the user's
//! opt-in intent and performs a read-only capability preflight so a future
//! PipeWire-based implementation has somewhere safe to start. It never opens
//! a HID/USB device, connects to a real PipeWire instance, changes the
//! default sink, creates links, or spawns a DSP graph — see
//! `docs/protocol/audio-processing.md` and `AGENTS.md` for the boundaries
//! this module must not cross. The only supported mode name is the open,
//! vendor-neutral "binaural-stereo"; vendor names such as DTS or Quantum
//! Spatial must never appear here.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

pub const SCHEMA: u8 = 1;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MUTATION_LOCK_NAME: &str = "spatial.lock";

const FILTER_CHAIN_MODULE_CANDIDATES: [&str; 2] = [
    "/usr/lib/pipewire-0.3/libpipewire-module-filter-chain.so",
    "/usr/lib64/pipewire-0.3/libpipewire-module-filter-chain.so",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpatialMode {
    Off,
    BinauralStereo,
}

impl SpatialMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SpatialMode::Off => "off",
            SpatialMode::BinauralStereo => "binaural-stereo",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "off" => Ok(SpatialMode::Off),
            "binaural-stereo" => Ok(SpatialMode::BinauralStereo),
            other => Err(format!(
                "unsupported spatial mode `{other}`; only `off` and `binaural-stereo` are defined"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpatialProfile {
    pub schema: u8,
    pub enabled: bool,
    pub mode: SpatialMode,
}

impl Default for SpatialProfile {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            enabled: false,
            mode: SpatialMode::Off,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityReport {
    pub schema: u8,
    pub pipewire_binary_present: bool,
    pub filter_chain_module_present: bool,
    pub hrtf_dataset_present: bool,
    pub ready: bool,
    pub error: Option<String>,
}

pub fn config_directory() -> Result<PathBuf, String> {
    let root = match env::var_os("XDG_CONFIG_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(
            env::var_os("HOME").ok_or("HOME is not set; cannot locate spatial configuration")?,
        )
        .join(".config"),
    };
    Ok(root.join("jambalinux-soniccore"))
}

/// The user-provided open HRTF/binaural dataset directory. This build never
/// downloads, generates, or bundles a dataset here; the directory is only
/// inspected for presence.
pub fn hrtf_dataset_directory() -> Result<PathBuf, String> {
    Ok(config_directory()?.join("spatial").join("hrtf"))
}

/// Serializes the complete load -> validate -> save transaction.
/// The returned file must remain in scope until the transaction completes.
pub fn mutation_lock() -> Result<File, String> {
    let directory = config_directory()?;
    fs::create_dir_all(&directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    let path = directory.join(MUTATION_LOCK_NAME);
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    lock.lock()
        .map_err(|error| format!("could not lock {}: {error}", path.display()))?;
    Ok(lock)
}

fn config_path() -> Result<PathBuf, String> {
    Ok(config_directory()?.join("spatial.json"))
}

fn validate(profile: SpatialProfile) -> Result<SpatialProfile, String> {
    if profile.schema != SCHEMA {
        return Err("unsupported spatial profile".into());
    }
    if profile.enabled && profile.mode == SpatialMode::Off {
        return Err("spatial cannot be enabled while its mode is `off`".into());
    }
    Ok(profile)
}

fn load_from_path(path: &Path) -> Result<SpatialProfile, String> {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str::<SpatialProfile>(&content)
            .map_err(|error| format!("{}: {error}", path.display()))
            .and_then(validate),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SpatialProfile::default()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn temporary_path(path: &Path, sequence: u64) -> Result<PathBuf, String> {
    let directory = path
        .parent()
        .ok_or("invalid spatial configuration path")?;
    let file_name = path
        .file_name()
        .ok_or("invalid spatial configuration path")?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(format!(".tmp-{}-{sequence}", std::process::id()));
    Ok(directory.join(temporary_name))
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    let directory = path
        .parent()
        .ok_or("invalid spatial configuration path")?;
    fs::create_dir_all(directory).map_err(|error| format!("{}: {error}", directory.display()))?;

    let (temporary_path, mut temporary_file) = loop {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary_path = temporary_path(path, sequence)?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => break (temporary_path, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("{}: {error}", temporary_path.display())),
        }
    };

    let write_result = temporary_file
        .write_all(content)
        .and_then(|()| temporary_file.sync_all());
    drop(temporary_file);

    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("{}: {error}", temporary_path.display()));
    }

    if let Err(error) = fs::rename(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("{}: {error}", path.display()));
    }

    Ok(())
}

fn save_to_path(profile: &SpatialProfile, path: &Path) -> Result<(), String> {
    let profile = validate(profile.clone())?;
    let content = serde_json::to_string_pretty(&profile).map_err(|error| error.to_string())?;
    atomic_write(path, format!("{content}\n").as_bytes())
}

pub fn load() -> Result<SpatialProfile, String> {
    load_from_path(&config_path()?)
}

pub fn save(profile: &SpatialProfile) -> Result<(), String> {
    save_to_path(profile, &config_path()?)
}

fn binary_present(name: &str) -> bool {
    let path_var = match env::var_os("PATH") {
        Some(value) => value,
        None => return false,
    };
    env::split_paths(&path_var).any(|directory| directory.join(name).is_file())
}

fn filter_chain_module_present() -> bool {
    FILTER_CHAIN_MODULE_CANDIDATES
        .iter()
        .any(|candidate| Path::new(candidate).is_file())
}

fn hrtf_dataset_present_at(directory: &Path) -> bool {
    fs::read_dir(directory)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

fn capability_report(
    pipewire_binary_present: bool,
    filter_chain_module_present: bool,
    hrtf_dataset_present: bool,
) -> CapabilityReport {
    let mut missing = Vec::new();
    if !pipewire_binary_present {
        missing.push("the `pipewire` binary was not found on PATH");
    }
    if !filter_chain_module_present {
        missing.push("no PipeWire filter-chain module was found under the known module directories");
    }
    if !hrtf_dataset_present {
        missing.push(
            "no open HRTF/binaural dataset was found under the spatial configuration directory",
        );
    }
    let ready = missing.is_empty();
    let error = if ready {
        None
    } else {
        Some(format!(
            "spatial capability unavailable: {}. This build never downloads or installs \
             these components automatically; place them manually and re-run the preflight.",
            missing.join("; ")
        ))
    };
    CapabilityReport {
        schema: SCHEMA,
        pipewire_binary_present,
        filter_chain_module_present,
        hrtf_dataset_present,
        ready,
        error,
    }
}

/// Read-only capability preflight. It inspects `PATH` and well-known
/// filesystem locations only; it never opens a PipeWire connection, executes
/// `pipewire`/`pw-cli`, or touches the default sink, routing, or DSP graph.
pub fn preflight() -> Result<CapabilityReport, String> {
    let hrtf_dataset_present = hrtf_dataset_present_at(&hrtf_dataset_directory()?);
    Ok(capability_report(
        binary_present("pipewire"),
        filter_chain_module_present(),
        hrtf_dataset_present,
    ))
}

fn enabled_profile(
    profile: &SpatialProfile,
    enabled: bool,
    capability: &CapabilityReport,
) -> Result<SpatialProfile, String> {
    if enabled && !capability.ready {
        return Err(capability
            .error
            .clone()
            .unwrap_or_else(|| "spatial capability unavailable".into()));
    }
    let mut next = profile.clone();
    next.enabled = enabled;
    if enabled && next.mode == SpatialMode::Off {
        next.mode = SpatialMode::BinauralStereo;
    }
    validate(next)
}

fn moded_profile(profile: &SpatialProfile, mode: SpatialMode) -> Result<SpatialProfile, String> {
    let mut next = profile.clone();
    next.mode = mode;
    if mode == SpatialMode::Off {
        next.enabled = false;
    }
    validate(next)
}

/// Sets the explicit spatial gate. Enabling requires the capability preflight
/// to report readiness; this only records intent in the profile and performs
/// no PipeWire connection, routing change, or DSP setup of any kind.
pub fn set_enabled(enabled: bool) -> Result<SpatialProfile, String> {
    let _lock = mutation_lock()?;
    let profile = load()?;
    let capability = preflight()?;
    let next = enabled_profile(&profile, enabled, &capability)?;
    save(&next)?;
    Ok(next)
}

pub fn set_mode(mode: SpatialMode) -> Result<SpatialProfile, String> {
    let _lock = mutation_lock()?;
    let profile = load()?;
    let next = moded_profile(&profile, mode)?;
    save(&next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn new() -> Self {
            loop {
                let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = env::temp_dir().join(format!(
                    "jambalinux-spatial-test-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create temporary test directory: {error}"),
                }
            }
        }

        fn profile_path(&self) -> PathBuf {
            self.0.join("spatial.json")
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn default_profile_is_off_and_disabled() {
        let profile = SpatialProfile::default();
        assert!(!profile.enabled);
        assert_eq!(profile.mode, SpatialMode::Off);
    }

    #[test]
    fn mode_parses_only_the_open_binaural_names() {
        assert_eq!(SpatialMode::parse("off"), Ok(SpatialMode::Off));
        assert_eq!(
            SpatialMode::parse("binaural-stereo"),
            Ok(SpatialMode::BinauralStereo)
        );
        assert!(SpatialMode::parse("quantum-spatial").is_err());
        assert!(SpatialMode::parse("dts").is_err());
        assert!(SpatialMode::parse("").is_err());
    }

    #[test]
    fn mode_names_never_leak_vendor_branding() {
        for mode in [SpatialMode::Off, SpatialMode::BinauralStereo] {
            let name = mode.as_str().to_ascii_lowercase();
            assert!(!name.contains("dts"));
            assert!(!name.contains("quantum"));
        }
    }

    #[test]
    fn validation_rejects_unknown_schema_and_enabled_off() {
        let profile = SpatialProfile {
            schema: 2,
            ..SpatialProfile::default()
        };
        assert!(validate(profile).is_err());

        let profile = SpatialProfile {
            schema: SCHEMA,
            enabled: true,
            mode: SpatialMode::Off,
        };
        assert!(validate(profile).is_err());

        let profile = SpatialProfile {
            schema: SCHEMA,
            enabled: true,
            mode: SpatialMode::BinauralStereo,
        };
        assert!(validate(profile).is_ok());
    }

    #[test]
    fn capability_report_is_ready_only_when_everything_is_present() {
        let ready = capability_report(true, true, true);
        assert!(ready.ready);
        assert!(ready.error.is_none());

        let missing_pipewire = capability_report(false, true, true);
        assert!(!missing_pipewire.ready);
        assert!(missing_pipewire
            .error
            .as_deref()
            .unwrap()
            .contains("pipewire"));

        let missing_module = capability_report(true, false, true);
        assert!(!missing_module.ready);
        assert!(missing_module
            .error
            .as_deref()
            .unwrap()
            .contains("filter-chain"));

        let missing_dataset = capability_report(true, true, false);
        assert!(!missing_dataset.ready);
        assert!(missing_dataset
            .error
            .as_deref()
            .unwrap()
            .contains("HRTF"));
    }

    #[test]
    fn hrtf_dataset_presence_reflects_the_filesystem_only() {
        let directory = TemporaryDirectory::new();
        let dataset_directory = directory.0.join("hrtf");
        assert!(!hrtf_dataset_present_at(&dataset_directory));

        fs::create_dir_all(&dataset_directory).expect("create dataset directory");
        assert!(!hrtf_dataset_present_at(&dataset_directory));

        fs::write(dataset_directory.join("left.wav"), b"placeholder")
            .expect("write placeholder dataset file");
        assert!(hrtf_dataset_present_at(&dataset_directory));
    }

    #[test]
    fn enabling_without_capability_returns_an_actionable_error_and_does_not_change_the_profile() {
        let profile = SpatialProfile::default();
        let not_ready = capability_report(false, false, false);
        let error = enabled_profile(&profile, true, &not_ready)
            .expect_err("enabling without capability must fail");
        assert!(error.contains("spatial capability unavailable"));
        assert!(error.contains("pipewire"));
    }

    #[test]
    fn enabling_with_capability_selects_the_open_binaural_mode() {
        let profile = SpatialProfile::default();
        let ready = capability_report(true, true, true);
        let next = enabled_profile(&profile, true, &ready).expect("enable with capability");
        assert!(next.enabled);
        assert_eq!(next.mode, SpatialMode::BinauralStereo);
    }

    #[test]
    fn disabling_never_consults_the_capability_report() {
        let profile = SpatialProfile {
            schema: SCHEMA,
            enabled: true,
            mode: SpatialMode::BinauralStereo,
        };
        let not_ready = capability_report(false, false, false);
        let next = enabled_profile(&profile, false, &not_ready).expect("disable always succeeds");
        assert!(!next.enabled);
    }

    #[test]
    fn selecting_off_mode_clears_the_gate() {
        let profile = SpatialProfile {
            schema: SCHEMA,
            enabled: true,
            mode: SpatialMode::BinauralStereo,
        };
        let next = moded_profile(&profile, SpatialMode::Off).expect("select off mode");
        assert!(!next.enabled);
        assert_eq!(next.mode, SpatialMode::Off);
    }

    #[test]
    fn save_normalizes_and_round_trips_through_an_atomic_replacement() {
        use std::io::Read;

        let directory = TemporaryDirectory::new();
        let path = directory.profile_path();
        let original = SpatialProfile::default();
        save_to_path(&original, &path).expect("save original profile");

        // Keeping the old inode open proves the path is replaced rather than
        // truncated in place: an existing reader still sees the old profile.
        let mut old_file = fs::File::open(&path).expect("open original profile");
        let changed = SpatialProfile {
            schema: SCHEMA,
            enabled: true,
            mode: SpatialMode::BinauralStereo,
        };
        save_to_path(&changed, &path).expect("atomically replace profile");

        let loaded = load_from_path(&path).expect("load replacement profile");
        assert_eq!(loaded, changed);

        let mut old_content = String::new();
        old_file
            .read_to_string(&mut old_content)
            .expect("read replaced profile through old handle");
        let old_profile: SpatialProfile =
            serde_json::from_str(&old_content).expect("parse original profile");
        assert_eq!(old_profile, original);
    }

    #[test]
    fn rejected_profile_preserves_the_previously_saved_file() {
        let directory = TemporaryDirectory::new();
        let path = directory.profile_path();
        save_to_path(&SpatialProfile::default(), &path).expect("save original profile");
        let original_content = fs::read(&path).expect("read original profile");

        let invalid = SpatialProfile {
            schema: SCHEMA,
            enabled: true,
            mode: SpatialMode::Off,
        };
        assert!(save_to_path(&invalid, &path).is_err());

        assert_eq!(
            fs::read(&path).expect("read preserved profile"),
            original_content
        );
    }

    #[test]
    fn missing_profile_file_loads_the_disabled_default() {
        let directory = TemporaryDirectory::new();
        let path = directory.profile_path();
        assert_eq!(load_from_path(&path).expect("default profile"), SpatialProfile::default());
    }
}
