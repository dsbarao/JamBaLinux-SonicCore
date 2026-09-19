//! Host-side equalizer profile storage.
//!
//! The Quantum 810 equalizer is not a HID setting: controlled captures show
//! that QuantumENGINE applies it in the host audio stack.  This module keeps
//! the user-editable profile separate from device state so a future PipeWire
//! backend can consume it without ever sending a USB report.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

pub const BANDS_HZ: [u32; 10] = [31, 62, 125, 250, 500, 1_000, 2_000, 4_000, 8_000, 16_000];
pub const MIN_GAIN_DB: f32 = -12.0;
pub const MAX_GAIN_DB: f32 = 12.0;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MUTATION_LOCK_NAME: &str = "equalizer.lock";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub frequency_hz: u32,
    pub gain_db: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqualizerProfile {
    pub schema: u8,
    pub bands: Vec<Band>,
}

impl Default for EqualizerProfile {
    fn default() -> Self {
        Self {
            schema: 1,
            bands: BANDS_HZ
                .into_iter()
                .map(|frequency_hz| Band {
                    frequency_hz,
                    gain_db: 0.0,
                })
                .collect(),
        }
    }
}

pub fn config_directory() -> Result<PathBuf, String> {
    let root = match env::var_os("XDG_CONFIG_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(
            env::var_os("HOME").ok_or("HOME is not set; cannot locate equalizer configuration")?,
        )
        .join(".config"),
    };
    Ok(root.join("jambalinux-soniccore"))
}

/// Serializes the complete load -> live DSP update -> profile save transaction.
/// The returned file must remain in scope until any rollback has completed.
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
    Ok(config_directory()?.join("equalizer.json"))
}

fn normalized_gain(gain_db: f32) -> Result<f32, String> {
    if !gain_db.is_finite() || !(MIN_GAIN_DB..=MAX_GAIN_DB).contains(&gain_db) {
        return Err(format!(
            "equalizer gain must be between {MIN_GAIN_DB:.0} and {MAX_GAIN_DB:.0} dB"
        ));
    }
    Ok((gain_db * 10.0).round() / 10.0)
}

fn validate(profile: EqualizerProfile) -> Result<EqualizerProfile, String> {
    if profile.schema != 1 || profile.bands.len() != BANDS_HZ.len() {
        return Err("unsupported equalizer profile".into());
    }
    let mut normalized = profile;
    for (band, expected_frequency) in normalized.bands.iter_mut().zip(BANDS_HZ) {
        if band.frequency_hz != expected_frequency {
            return Err("equalizer profile has unsupported bands".into());
        }
        band.gain_db = normalized_gain(band.gain_db)?;
    }
    Ok(normalized)
}

fn load_from_path(path: &Path) -> Result<EqualizerProfile, String> {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str::<EqualizerProfile>(&content)
            .map_err(|error| format!("{}: {error}", path.display()))
            .and_then(validate),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(EqualizerProfile::default())
        }
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn temporary_path(path: &Path, sequence: u64) -> Result<PathBuf, String> {
    let directory = path
        .parent()
        .ok_or("invalid equalizer configuration path")?;
    let file_name = path
        .file_name()
        .ok_or("invalid equalizer configuration path")?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(format!(".tmp-{}-{sequence}", std::process::id()));
    Ok(directory.join(temporary_name))
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    let directory = path
        .parent()
        .ok_or("invalid equalizer configuration path")?;
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

fn save_to_path(profile: &EqualizerProfile, path: &Path) -> Result<(), String> {
    let profile = validate(profile.clone())?;
    let content = serde_json::to_string_pretty(&profile).map_err(|error| error.to_string())?;
    atomic_write(path, format!("{content}\n").as_bytes())
}

pub fn load() -> Result<EqualizerProfile, String> {
    load_from_path(&config_path()?)
}

pub fn save(profile: &EqualizerProfile) -> Result<(), String> {
    save_to_path(profile, &config_path()?)
}

/// Builds a validated profile with one changed band without persisting it.
///
/// Callers that also update a live DSP can apply the returned profile first
/// and call [`save`] only after the DSP transaction succeeds.
pub fn updated_profile(
    profile: &EqualizerProfile,
    frequency_hz: u32,
    gain_db: f32,
) -> Result<EqualizerProfile, String> {
    let gain_db = normalized_gain(gain_db)?;
    let mut profile = validate(profile.clone())?;
    let band = profile
        .bands
        .iter_mut()
        .find(|band| band.frequency_hz == frequency_hz)
        .ok_or("unsupported equalizer frequency")?;
    band.gain_db = gain_db;
    Ok(profile)
}

/// Builds the canonical flat profile without persisting it.
pub fn reset_profile() -> EqualizerProfile {
    EqualizerProfile::default()
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
                    "jambalinux-equalizer-test-{}-{sequence}",
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
            self.0.join("equalizer.json")
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn default_profile_uses_the_documented_ten_bands() {
        let profile = EqualizerProfile::default();
        assert_eq!(profile.bands.len(), 10);
        assert_eq!(profile.bands[0].frequency_hz, 31);
        assert_eq!(profile.bands[9].frequency_hz, 16_000);
        assert!(profile.bands.iter().all(|band| band.gain_db == 0.0));
    }

    #[test]
    fn gain_is_bounded_and_stored_to_one_decimal_place() {
        assert_eq!(normalized_gain(3.17), Ok(3.2));
        assert_eq!(normalized_gain(MIN_GAIN_DB), Ok(MIN_GAIN_DB));
        assert_eq!(normalized_gain(MAX_GAIN_DB), Ok(MAX_GAIN_DB));
        assert!(normalized_gain(-12.1).is_err());
        assert!(normalized_gain(12.1).is_err());
        assert!(normalized_gain(f32::NAN).is_err());
        assert!(normalized_gain(f32::INFINITY).is_err());
    }

    #[test]
    fn validation_rejects_unknown_schema_shape_and_band_order() {
        let profile = EqualizerProfile {
            schema: 2,
            ..EqualizerProfile::default()
        };
        assert!(validate(profile).is_err());

        let mut profile = EqualizerProfile::default();
        profile.bands.pop();
        assert!(validate(profile).is_err());

        let mut profile = EqualizerProfile::default();
        profile.bands.swap(0, 1);
        assert!(validate(profile).is_err());
    }

    #[test]
    fn updated_profile_is_normalized_without_mutating_the_source() {
        let mut source = EqualizerProfile::default();
        source.bands[0].gain_db = 1.24;

        let updated = updated_profile(&source, 500, 3.17).expect("build updated profile");

        assert_eq!(source.bands[0].gain_db, 1.24);
        assert_eq!(source.bands[4].gain_db, 0.0);
        assert_eq!(updated.bands[0].gain_db, 1.2);
        assert_eq!(updated.bands[4].gain_db, 3.2);
        assert!(updated_profile(&source, 123, 0.0).is_err());
    }

    #[test]
    fn reset_profile_is_flat_without_touching_an_existing_profile() {
        let mut existing = EqualizerProfile::default();
        existing.bands[4].gain_db = 6.0;

        let reset = reset_profile();

        assert_eq!(existing.bands[4].gain_db, 6.0);
        assert!(reset.bands.iter().all(|band| band.gain_db == 0.0));
    }

    #[test]
    fn save_normalizes_and_round_trips_through_an_atomic_replacement() {
        use std::io::Read;

        let directory = TemporaryDirectory::new();
        let path = directory.profile_path();
        let original = EqualizerProfile::default();
        save_to_path(&original, &path).expect("save original profile");

        // Keeping the old inode open proves the path is replaced rather than
        // truncated in place: an existing reader still sees the old profile.
        let mut old_file = fs::File::open(&path).expect("open original profile");
        let mut changed = original.clone();
        changed.bands[4].gain_db = 3.17;
        save_to_path(&changed, &path).expect("atomically replace profile");

        let loaded = load_from_path(&path).expect("load replacement profile");
        assert_eq!(loaded.bands[4].gain_db, 3.2);

        let mut old_content = String::new();
        old_file
            .read_to_string(&mut old_content)
            .expect("read replaced profile through old handle");
        let old_profile: EqualizerProfile =
            serde_json::from_str(&old_content).expect("parse original profile");
        assert_eq!(old_profile, original);

        let entries = fs::read_dir(&directory.0)
            .expect("read temporary directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("read every directory entry");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path(), path);
    }

    #[test]
    fn rejected_profile_preserves_the_previously_saved_file() {
        let directory = TemporaryDirectory::new();
        let path = directory.profile_path();
        save_to_path(&EqualizerProfile::default(), &path).expect("save original profile");
        let original_content = fs::read(&path).expect("read original profile");

        let mut invalid = EqualizerProfile::default();
        invalid.bands[0].frequency_hz = 32;
        assert!(save_to_path(&invalid, &path).is_err());

        assert_eq!(
            fs::read(&path).expect("read preserved profile"),
            original_content
        );
    }
}
