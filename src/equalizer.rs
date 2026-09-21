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
pub const PROFILE_SCHEMA: u8 = 2;
pub const MAX_CUSTOM_PROFILE_NAME_LENGTH: usize = 64;

/// A complete 10-band profile transcribed from the original software UI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqualizerPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub gains_db: [f32; 10],
}

pub const PRESETS: [EqualizerPreset; 17] = [
    EqualizerPreset {
        id: "flat",
        name: "Flat",
        gains_db: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    },
    EqualizerPreset {
        id: "bass-boost",
        name: "Bass Boost",
        gains_db: [6.0, 6.0, 4.0, 2.0, 0.0, 0.0, 0.0, 2.0, 1.0, 0.0],
    },
    EqualizerPreset {
        id: "cinematic",
        name: "Cinematic",
        gains_db: [4.0, 3.0, 2.0, 0.0, -1.0, 1.0, 2.0, -1.0, -2.0, -2.0],
    },
    EqualizerPreset {
        id: "fps",
        name: "FPS",
        gains_db: [-5.0, -3.0, -1.0, 0.0, -1.0, 1.0, 4.0, 2.0, -1.0, 0.0],
    },
    EqualizerPreset {
        id: "moba",
        name: "MOBA",
        gains_db: [3.0, 4.0, 2.0, 1.0, -1.0, -1.0, 1.0, 2.0, -3.0, -3.0],
    },
    EqualizerPreset {
        id: "rpg",
        name: "RPG",
        gains_db: [3.0, 3.0, 3.0, 0.0, -2.0, -2.0, -2.0, 1.0, 2.0, -2.0],
    },
    EqualizerPreset {
        id: "apex-legends",
        name: "Apex Legends",
        gains_db: [-6.0, 3.0, 2.0, -3.0, 2.0, 1.0, 2.0, 4.0, 1.0, -4.0],
    },
    EqualizerPreset {
        id: "cs2",
        name: "CS2",
        gains_db: [-6.0, -4.0, -3.0, -2.0, 2.0, 3.0, 4.0, 4.0, 3.0, 0.0],
    },
    EqualizerPreset {
        id: "dota-2",
        name: "Dota 2",
        gains_db: [4.0, 3.0, 1.0, -4.0, -1.0, 3.0, 4.0, 1.0, 3.0, -4.0],
    },
    EqualizerPreset {
        id: "fortnite",
        name: "Fortnite",
        gains_db: [-4.0, 3.0, 2.0, -3.0, -4.0, 3.0, 3.0, 4.0, 2.0, -4.0],
    },
    EqualizerPreset {
        id: "gta-5",
        name: "GTA 5",
        gains_db: [-1.0, 1.0, 2.0, -3.0, 3.0, 4.0, 3.0, 3.0, 2.0, -2.0],
    },
    EqualizerPreset {
        id: "lol",
        name: "LoL",
        gains_db: [2.0, 3.0, 1.0, -3.0, -4.0, 1.0, 2.0, 4.0, 1.0, -3.0],
    },
    EqualizerPreset {
        id: "pubg",
        name: "PUBG",
        gains_db: [-1.0, -2.0, -1.0, -1.0, 2.0, 3.0, 0.0, 5.0, 4.0, -2.0],
    },
    EqualizerPreset {
        id: "wow",
        name: "WoW",
        gains_db: [3.0, 2.0, 2.0, -1.0, -2.0, -1.0, 1.0, 3.0, -2.0, -2.0],
    },
    EqualizerPreset {
        id: "escape-from-tarkov",
        name: "Escape from Tarkov",
        gains_db: [-4.0, 3.0, -3.0, 6.0, 3.0, 1.0, 3.0, 6.0, 2.0, 1.0],
    },
    EqualizerPreset {
        id: "ln3-immersion",
        name: "LN3 Immersion",
        gains_db: [4.0, 5.0, 4.0, 1.0, -3.0, -2.0, 1.0, 3.0, 1.0, -2.0],
    },
    EqualizerPreset {
        id: "ln3-thrill",
        name: "LN3 Thrill",
        gains_db: [1.0, 0.0, -1.0, -5.0, 2.0, 1.0, 3.0, 4.0, 2.0, 1.0],
    },
];

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MUTATION_LOCK_NAME: &str = "equalizer.lock";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub frequency_hz: u32,
    pub gain_db: f32,
}

/// A user-owned named profile. Factory presets are deliberately not represented
/// here: they remain the immutable [`PRESETS`] table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomEqualizerProfile {
    pub id: String,
    pub name: String,
    pub bands: Vec<Band>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqualizerProfile {
    pub schema: u8,
    /// The bands consumed directly by the PipeWire DSP.
    pub bands: Vec<Band>,
    /// Factory or custom profile currently selected, or `None` for unsaved
    /// manual settings.
    #[serde(default)]
    pub active_profile_id: Option<String>,
    #[serde(default)]
    pub custom_profiles: Vec<CustomEqualizerProfile>,
}

impl Default for EqualizerProfile {
    fn default() -> Self {
        Self {
            schema: PROFILE_SCHEMA,
            bands: BANDS_HZ
                .into_iter()
                .map(|frequency_hz| Band {
                    frequency_hz,
                    gain_db: 0.0,
                })
                .collect(),
            active_profile_id: Some("flat".into()),
            custom_profiles: Vec::new(),
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

fn validate_bands(bands: &mut [Band]) -> Result<(), String> {
    if bands.len() != BANDS_HZ.len() {
        return Err("equalizer profile must contain exactly ten bands".into());
    }
    for (band, expected_frequency) in bands.iter_mut().zip(BANDS_HZ) {
        if band.frequency_hz != expected_frequency {
            return Err("equalizer profile has unsupported bands".into());
        }
        band.gain_db = normalized_gain(band.gain_db)?;
    }
    Ok(())
}

fn bands_match(left: &[Band], right: &[Band]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.frequency_hz == right.frequency_hz && (left.gain_db - right.gain_db).abs() < 0.05
        })
}

fn normalize_name(name: &str) -> Result<String, String> {
    if name.chars().any(char::is_control) {
        return Err("custom equalizer profile name cannot contain control characters".into());
    }
    let name = name.trim();
    if name.is_empty() {
        return Err("custom equalizer profile name cannot be empty".into());
    }
    if name.chars().count() > MAX_CUSTOM_PROFILE_NAME_LENGTH {
        return Err(format!(
            "custom equalizer profile name must be at most {MAX_CUSTOM_PROFILE_NAME_LENGTH} characters"
        ));
    }
    Ok(name.to_owned())
}

fn name_key(name: &str) -> String {
    name.to_lowercase()
}

/// Returns whether an identifier belongs to an immutable built-in preset.
pub fn is_factory_profile_id(id: &str) -> bool {
    PRESETS.iter().any(|preset| preset.id == id)
}

fn preset_bands(preset: &EqualizerPreset) -> Vec<Band> {
    BANDS_HZ
        .into_iter()
        .zip(preset.gains_db)
        .map(|(frequency_hz, gain_db)| Band {
            frequency_hz,
            gain_db,
        })
        .collect()
}

fn profile_bands_by_id(profile: &EqualizerProfile, id: &str) -> Option<Vec<Band>> {
    PRESETS
        .iter()
        .find(|preset| preset.id == id)
        .map(preset_bands)
        .or_else(|| {
            profile
                .custom_profiles
                .iter()
                .find(|custom| custom.id == id)
                .map(|custom| custom.bands.clone())
        })
}

/// Identifier reserved for the single custom profile a schema 1 file migrates
/// to. Loading is a pure read, so the id may not depend on the process, a
/// counter or the clock: two consecutive readers must agree on it.
const LEGACY_CUSTOM_PROFILE_ID_PREFIX: &str = "custom-legacy-";
const LEGACY_CUSTOM_PROFILE_NAME: &str = "Personalizado";

/// Picks the reserved legacy identifier, deriving the numeric suffix only from
/// the profiles already present in the file being migrated.
///
/// A well-formed schema 1 file carries no custom profiles, so `existing` is
/// normally empty and the id is always `custom-legacy-1`. The scan is kept for
/// hand-edited files in which the schema 2 field leaked into a schema 1
/// document: those entries are not carried over, and refusing to reuse one of
/// their identifiers keeps a single id from denoting two different profiles
/// across the pre- and post-migration files.
fn legacy_custom_id(existing: &[CustomEqualizerProfile]) -> String {
    let mut suffix = 1u32;
    loop {
        let id = format!("{LEGACY_CUSTOM_PROFILE_ID_PREFIX}{suffix}");
        if !existing
            .iter()
            .any(|custom| custom.id == id || name_key(&custom.name) == name_key(&id))
        {
            return id;
        }
        suffix += 1;
    }
}

fn migrate_schema_one(mut profile: EqualizerProfile) -> Result<EqualizerProfile, String> {
    validate_bands(&mut profile.bands)?;
    let preset = PRESETS
        .iter()
        .find(|preset| bands_match(&profile.bands, &preset_bands(preset)));
    let migrated = match preset {
        Some(preset) => EqualizerProfile {
            schema: PROFILE_SCHEMA,
            bands: profile.bands,
            active_profile_id: Some(preset.id.to_owned()),
            custom_profiles: Vec::new(),
        },
        // Schema 1 had no named profiles, so bands that match no factory preset
        // become the one custom profile the migrated file selects.
        None => {
            let id = legacy_custom_id(&profile.custom_profiles);
            EqualizerProfile {
                schema: PROFILE_SCHEMA,
                bands: profile.bands.clone(),
                active_profile_id: Some(id.clone()),
                custom_profiles: vec![CustomEqualizerProfile {
                    id,
                    name: LEGACY_CUSTOM_PROFILE_NAME.to_owned(),
                    bands: profile.bands,
                }],
            }
        }
    };
    // Hand the result through the schema 2 checks so a migrated file is held to
    // the same id, name and active-selection invariants as a saved one.
    validate(migrated)
}

fn validate(profile: EqualizerProfile) -> Result<EqualizerProfile, String> {
    if profile.schema == 1 {
        return migrate_schema_one(profile);
    }
    if profile.schema != PROFILE_SCHEMA {
        return Err("unsupported equalizer profile schema".into());
    }

    let mut normalized = profile;
    validate_bands(&mut normalized.bands)?;
    let mut names = std::collections::HashSet::new();
    let mut ids = std::collections::HashSet::new();
    for custom in &mut normalized.custom_profiles {
        if custom.id.is_empty()
            || is_factory_profile_id(&custom.id)
            || !ids.insert(custom.id.clone())
        {
            return Err("custom equalizer profile has an invalid or duplicate id".into());
        }
        custom.name = normalize_name(&custom.name)?;
        if !names.insert(name_key(&custom.name)) {
            return Err("custom equalizer profile names must be unique".into());
        }
        validate_bands(&mut custom.bands)?;
    }
    if let Some(id) = normalized.active_profile_id.as_deref() {
        let expected = profile_bands_by_id(&normalized, id)
            .ok_or("active equalizer profile id does not exist")?;
        if !bands_match(&normalized.bands, &expected) {
            return Err("active equalizer profile bands do not match the selected profile".into());
        }
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
    let mut candidate = profile.clone();
    if candidate
        .active_profile_id
        .as_deref()
        .and_then(|id| profile_bands_by_id(&candidate, id))
        .is_some_and(|expected| !bands_match(&candidate.bands, &expected))
    {
        candidate.active_profile_id = None;
    }
    let mut profile = validate(candidate)?;
    let stored_profile = profile.clone();
    let band = profile
        .bands
        .iter_mut()
        .find(|band| band.frequency_hz == frequency_hz)
        .ok_or("unsupported equalizer frequency")?;
    band.gain_db = gain_db;
    if let Some(id) = profile.active_profile_id.clone()
        && profile.custom_profiles.iter().any(|custom| custom.id == id)
    {
        let bands = profile.bands.clone();
        return update_custom_profile(&stored_profile, &id, bands);
    }

    // A changed factory preset becomes unsaved manual settings rather than
    // changing the immutable preset definition.
    profile.active_profile_id = None;
    Ok(profile)
}

/// Builds the canonical flat profile without persisting it.
pub fn reset_profile() -> EqualizerProfile {
    EqualizerProfile::default()
}

/// Builds one of the complete profiles transcribed from the reference images.
pub fn preset_profile(id: &str) -> Result<EqualizerProfile, String> {
    select_profile(&EqualizerProfile::default(), id)
}

/// Identifies a profile without persisting a separate, stale preset id.
pub fn matching_preset(profile: &EqualizerProfile) -> Option<&'static EqualizerPreset> {
    if profile.bands.len() != BANDS_HZ.len() {
        return None;
    }
    PRESETS.iter().find(|preset| {
        profile.bands.iter().zip(BANDS_HZ).zip(preset.gains_db).all(
            |((band, frequency_hz), gain_db)| {
                band.frequency_hz == frequency_hz && (band.gain_db - gain_db).abs() < 0.05
            },
        )
    })
}

fn generated_custom_id(profile: &EqualizerProfile) -> String {
    loop {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let id = format!("custom-{}-{sequence}", std::process::id());
        if !profile
            .custom_profiles
            .iter()
            .any(|custom| custom.id == id || name_key(&custom.name) == name_key(&id))
        {
            return id;
        }
    }
}

fn custom_profile_index(profile: &EqualizerProfile, id: &str) -> Result<usize, String> {
    if is_factory_profile_id(id) {
        return Err("factory equalizer presets are immutable".into());
    }
    profile
        .custom_profiles
        .iter()
        .position(|custom| custom.id == id)
        .ok_or_else(|| format!("unknown custom equalizer profile `{id}`"))
}

/// Adds a named custom profile from the currently active DSP bands and selects it.
pub fn create_custom_profile(
    profile: &EqualizerProfile,
    name: &str,
) -> Result<EqualizerProfile, String> {
    let mut profile = validate(profile.clone())?;
    let id = generated_custom_id(&profile);
    profile.custom_profiles.push(CustomEqualizerProfile {
        id: id.clone(),
        // Set a temporary unique, valid name so the shared rename path can
        // perform all final name normalization and duplicate checks.
        name: id.clone(),
        bands: profile.bands.clone(),
    });
    let mut profile = rename_custom_profile(&profile, &id, name)?;
    profile.active_profile_id = Some(id);
    Ok(profile)
}

/// Selects an immutable factory preset or a user-owned custom profile.
pub fn select_profile(profile: &EqualizerProfile, id: &str) -> Result<EqualizerProfile, String> {
    let mut profile = validate(profile.clone())?;
    let bands = profile_bands_by_id(&profile, id)
        .ok_or_else(|| format!("unknown equalizer profile `{id}`"))?;
    profile.bands = bands;
    profile.active_profile_id = Some(id.to_owned());
    Ok(profile)
}

/// Replaces a custom profile's complete ten-band definition. Factory presets
/// cannot be updated through this API.
pub fn update_custom_profile(
    profile: &EqualizerProfile,
    id: &str,
    bands: Vec<Band>,
) -> Result<EqualizerProfile, String> {
    let profile = validate(profile.clone())?;
    let index = custom_profile_index(&profile, id)?;
    let mut bands = bands;
    validate_bands(&mut bands)?;
    let name = profile.custom_profiles[index].name.clone();
    let was_active = profile.active_profile_id.as_deref() == Some(id);

    // Reuse the deletion path so all mutations share factory-ID rejection and
    // active-selection cleanup before replacing the immutable ID in place.
    let mut profile = delete_custom_profile(&profile, id)?;
    profile.custom_profiles.insert(
        index,
        CustomEqualizerProfile {
            id: id.to_owned(),
            name,
            bands: bands.clone(),
        },
    );
    if was_active {
        profile.active_profile_id = Some(id.to_owned());
        profile.bands = bands;
    }
    Ok(profile)
}

/// Renames a custom profile. Name comparisons are case-insensitive.
pub fn rename_custom_profile(
    profile: &EqualizerProfile,
    id: &str,
    name: &str,
) -> Result<EqualizerProfile, String> {
    let mut profile = validate(profile.clone())?;
    let index = custom_profile_index(&profile, id)?;
    let name = normalize_name(name)?;
    if profile
        .custom_profiles
        .iter()
        .enumerate()
        .any(|(other, custom)| other != index && name_key(&custom.name) == name_key(&name))
    {
        return Err("a custom equalizer profile with that name already exists".into());
    }
    profile.custom_profiles[index].name = name;
    Ok(profile)
}

/// Deletes a user-owned custom profile. Factory preset IDs are rejected.
pub fn delete_custom_profile(
    profile: &EqualizerProfile,
    id: &str,
) -> Result<EqualizerProfile, String> {
    let mut profile = validate(profile.clone())?;
    let index = custom_profile_index(&profile, id)?;
    profile.custom_profiles.remove(index);
    if profile.active_profile_id.as_deref() == Some(id) {
        profile.active_profile_id = None;
    }
    Ok(profile)
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
            schema: PROFILE_SCHEMA + 1,
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
    fn presets_have_documented_shape_and_can_be_identified() {
        assert_eq!(PRESETS.len(), 17);
        for preset in PRESETS {
            let profile = preset_profile(preset.id).expect("build preset profile");
            assert_eq!(profile.bands.len(), BANDS_HZ.len());
            assert_eq!(
                matching_preset(&profile).map(|match_| match_.id),
                Some(preset.id)
            );
        }

        assert_eq!(
            preset_profile("bass-boost").expect("bass boost").bands[0].gain_db,
            6.0
        );
        assert!(preset_profile("not-a-preset").is_err());
    }

    #[test]
    fn a_manual_band_change_is_not_reported_as_a_preset() {
        let custom = updated_profile(&reset_profile(), 31, 1.0).expect("custom profile");
        assert!(matching_preset(&custom).is_none());
        assert_eq!(custom.active_profile_id, None);
    }

    #[test]
    fn schema_one_custom_profile_migrates_without_losing_its_gains() {
        let directory = TemporaryDirectory::new();
        let path = directory.profile_path();
        let legacy = r#"{
  "schema": 1,
  "bands": [
    {"frequency_hz":31,"gain_db":3.0}, {"frequency_hz":62,"gain_db":0.0},
    {"frequency_hz":125,"gain_db":0.0}, {"frequency_hz":250,"gain_db":0.0},
    {"frequency_hz":500,"gain_db":-2.0}, {"frequency_hz":1000,"gain_db":0.0},
    {"frequency_hz":2000,"gain_db":0.0}, {"frequency_hz":4000,"gain_db":0.0},
    {"frequency_hz":8000,"gain_db":0.0}, {"frequency_hz":16000,"gain_db":1.0}
  ]
}"#;
        fs::write(&path, legacy).expect("write legacy profile");

        let migrated = load_from_path(&path).expect("migrate legacy profile");
        assert_eq!(migrated.schema, PROFILE_SCHEMA);
        assert_eq!(
            migrated.active_profile_id.as_deref(),
            Some(migrated.custom_profiles[0].id.as_str())
        );
        assert_eq!(migrated.custom_profiles.len(), 1);
        assert_eq!(migrated.custom_profiles[0].id, "custom-legacy-1");
        assert_eq!(migrated.custom_profiles[0].name, "Personalizado");
        assert_eq!(migrated.custom_profiles[0].bands.len(), BANDS_HZ.len());
        assert_eq!(migrated.custom_profiles[0].bands, migrated.bands);
        assert_eq!(migrated.bands[0].gain_db, 3.0);
        assert_eq!(migrated.bands[4].gain_db, -2.0);

        save_to_path(&migrated, &path).expect("persist migrated profile");
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read persisted migration"))
                .expect("parse persisted migration");
        assert_eq!(persisted["schema"], PROFILE_SCHEMA);
    }

    #[test]
    fn repeated_schema_one_loads_are_pure_and_keep_a_stable_identifier() {
        let directory = TemporaryDirectory::new();
        let path = directory.profile_path();
        let legacy = r#"{
  "schema": 1,
  "bands": [
    {"frequency_hz":31,"gain_db":3.0}, {"frequency_hz":62,"gain_db":0.0},
    {"frequency_hz":125,"gain_db":0.0}, {"frequency_hz":250,"gain_db":0.0},
    {"frequency_hz":500,"gain_db":-2.0}, {"frequency_hz":1000,"gain_db":0.0},
    {"frequency_hz":2000,"gain_db":0.0}, {"frequency_hz":4000,"gain_db":0.0},
    {"frequency_hz":8000,"gain_db":0.0}, {"frequency_hz":16000,"gain_db":1.0}
  ]
}"#;
        // A second copy of the same content proves the identifier follows the
        // bytes rather than the reading process or the file it came from.
        let copy = directory.0.join("equalizer-copy.json");
        fs::write(&path, legacy).expect("write legacy profile");
        fs::write(&copy, legacy).expect("write a second copy of the legacy profile");

        let listing = |root: &Path| {
            let mut entries = fs::read_dir(root)
                .expect("read temporary directory")
                .map(|entry| entry.expect("read directory entry").path())
                .collect::<Vec<_>>();
            entries.sort();
            entries
        };
        let original_bytes = fs::read(&path).expect("read legacy bytes");
        let original_listing = listing(directory.0.as_path());

        let migrations = [&path, &path, &path, &copy]
            .map(|source| load_from_path(source).expect("migrate legacy profile"));
        for migrated in &migrations {
            assert_eq!(migrated.schema, PROFILE_SCHEMA);
            assert_eq!(migrated.custom_profiles.len(), 1);
            assert_eq!(migrated.custom_profiles[0].id, "custom-legacy-1");
            assert_eq!(migrated.custom_profiles[0].name, "Personalizado");
            assert_eq!(
                migrated.active_profile_id.as_deref(),
                Some(migrated.custom_profiles[0].id.as_str())
            );
            // The migration preserves every band, so a stable id never comes at
            // the cost of the gains it is supposed to name.
            assert_eq!(migrated.custom_profiles[0].bands, migrated.bands);
            assert_eq!(migrated.bands.len(), BANDS_HZ.len());
            assert_eq!(migrated.bands[0].gain_db, 3.0);
            assert_eq!(migrated.bands[4].gain_db, -2.0);
            assert_eq!(migrated.bands[9].gain_db, 1.0);
            assert_eq!(migrated, &migrations[0]);
            assert_eq!(
                serde_json::to_string_pretty(migrated).expect("render migrated profile"),
                serde_json::to_string_pretty(&migrations[0]).expect("render first migration")
            );
        }

        // Reading a legacy file must not write anything: neither the source
        // bytes nor a stray `.tmp-` replacement may appear.
        assert_eq!(
            fs::read(&path).expect("re-read legacy bytes"),
            original_bytes
        );
        assert_eq!(
            fs::read(&copy).expect("re-read copied bytes"),
            original_bytes
        );
        assert_eq!(listing(directory.0.as_path()), original_listing);
    }

    #[test]
    fn schema_one_factory_preset_migrates_to_its_factory_id() {
        let directory = TemporaryDirectory::new();
        let path = directory.profile_path();
        let legacy = r#"{
  "schema": 1,
  "bands": [
    {"frequency_hz":31,"gain_db":6.0}, {"frequency_hz":62,"gain_db":6.0},
    {"frequency_hz":125,"gain_db":4.0}, {"frequency_hz":250,"gain_db":2.0},
    {"frequency_hz":500,"gain_db":0.0}, {"frequency_hz":1000,"gain_db":0.0},
    {"frequency_hz":2000,"gain_db":0.0}, {"frequency_hz":4000,"gain_db":2.0},
    {"frequency_hz":8000,"gain_db":1.0}, {"frequency_hz":16000,"gain_db":0.0}
  ]
}"#;
        fs::write(&path, legacy).expect("write legacy preset");

        let migrated = load_from_path(&path).expect("migrate factory preset");
        assert_eq!(migrated.active_profile_id.as_deref(), Some("bass-boost"));
        assert!(migrated.custom_profiles.is_empty());
        assert_eq!(migrated.bands, preset_bands(&PRESETS[1]));
    }

    #[test]
    fn custom_names_are_normalized_unique_and_safe() {
        let profile = create_custom_profile(&reset_profile(), "  Meu Perfil  ")
            .expect("create named profile");
        assert_eq!(profile.custom_profiles[0].name, "Meu Perfil");
        assert!(create_custom_profile(&profile, "meu perfil").is_err());
        assert!(create_custom_profile(&profile, "\nunsafe").is_err());
        assert!(create_custom_profile(&profile, "   ").is_err());
        assert!(
            create_custom_profile(&profile, &"a".repeat(MAX_CUSTOM_PROFILE_NAME_LENGTH + 1))
                .is_err()
        );
    }

    #[test]
    fn custom_profile_crud_preserves_factory_immutability() {
        let created = create_custom_profile(&reset_profile(), "Jogos").expect("create profile");
        let id = created.custom_profiles[0].id.clone();
        let edited_bands = updated_profile(&created, 500, 4.0)
            .expect("edit active bands")
            .bands;
        let updated = update_custom_profile(&created, &id, edited_bands).expect("update custom");
        let selected = select_profile(&updated, &id).expect("select custom");
        assert_eq!(selected.active_profile_id.as_deref(), Some(id.as_str()));
        assert_eq!(selected.bands[4].gain_db, 4.0);

        let renamed = rename_custom_profile(&selected, &id, "Competitivo").expect("rename custom");
        assert_eq!(renamed.custom_profiles[0].name, "Competitivo");
        let deleted = delete_custom_profile(&renamed, &id).expect("delete custom");
        assert!(deleted.custom_profiles.is_empty());
        assert_eq!(deleted.active_profile_id, None);

        for operation in [
            update_custom_profile(&created, "flat", created.bands.clone()),
            rename_custom_profile(&created, "flat", "Nope"),
            delete_custom_profile(&created, "flat"),
        ] {
            assert!(operation.is_err());
        }
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
        changed.active_profile_id = None;
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

        let mut invalid_custom = EqualizerProfile::default();
        invalid_custom.custom_profiles.push(CustomEqualizerProfile {
            id: "custom-invalid".into(),
            name: "\u{7}bad".into(),
            bands: reset_profile().bands,
        });
        assert!(save_to_path(&invalid_custom, &path).is_err());
        assert_eq!(
            fs::read(&path).expect("read preserved profile after invalid custom"),
            original_content
        );
    }
}
