//! Experimental open spatial/binaural audio: configuration, state, and gate.
//!
//! This module stores the user's opt-in intent and performs the capability
//! preflight used by the lifecycle supervisor. It never opens a HID/USB
//! device; the separate `crate::spatial_pipewire` backend owns the optional
//! PipeWire graph and its narrowly-scoped routing. The only supported mode name is the open,
//! vendor-neutral "binaural-stereo"; vendor names such as DTS or Quantum
//! Spatial must never appear here.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Pure PipeWire filter-chain renderer for the spatial sink (Phase 2). Kept as
/// a child of this module and separate from `crate::pipewire` (the equalizer
/// backend) so the two never share routing logic.
pub mod graph;

pub const SCHEMA: u8 = 1;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
/// A valid HRIR dataset is immutable from the supervisor's point of view until
/// either input file's identity or timestamps change.  Retaining the report
/// here avoids re-reading and re-hashing up to 64 MiB on every 500 ms
/// supervisor preflight. Invalid reports and read errors are deliberately not
/// cached: a corrected dataset must be noticed on the very next preflight.
static DATASET_VALIDATION_CACHE: Mutex<Option<DatasetValidationCache>> = Mutex::new(None);
const MUTATION_LOCK_NAME: &str = "spatial.lock";
/// Serializes the short PipeWire wet/dry ramps issued by the CLI and the
/// persistent supervisor.  This is deliberately separate from
/// `MUTATION_LOCK_NAME`: the latter protects the profile file, while this one
/// protects the observable graph after an intent has been saved.
const MIX_LOCK_NAME: &str = "spatial-mix.lock";

#[cfg(test)]
static TEST_CONFIG_DIRECTORY: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
#[cfg(test)]
static TEST_CONFIG_DIRECTORY_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    pub dataset: DatasetReport,
    pub ready: bool,
    pub error: Option<String>,
}

pub fn config_directory() -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(directory) = TEST_CONFIG_DIRECTORY
        .lock()
        .map_err(|_| "test spatial configuration directory lock is poisoned")?
        .clone()
    {
        return Ok(directory);
    }

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
/// downloads, generates, or bundles a dataset here; it only reads the manifest
/// and HRIR the user placed here and validates them (see
/// [`validate_dataset_directory`]).
pub fn hrtf_dataset_directory() -> Result<PathBuf, String> {
    Ok(config_directory()?.join("spatial").join("hrtf"))
}

// ---------------------------------------------------------------------------
// Open HRIR dataset contract and validation (Phase 1)
//
// The dataset directory must provide (at least) these two regular files, which
// are the only entries this build reads; any other files present are ignored:
//   - `manifest.json`: neutral, vendor-free metadata plus the expected SHA-256;
//   - `hrir.wav`: a 14-channel WAV compatible with the layout used by the
//     official PipeWire `sink-virtual-surround-7.1-hesuvi.conf` example.
//
// Everything here is a pure function of bytes or of a caller-supplied
// directory path, so it is exercised entirely with synthetic temporary files
// in tests. Validation never opens a PipeWire connection, spawns a graph,
// downloads anything, or mutates the dataset. Reads are size-bounded so a
// hostile or corrupt file cannot exhaust memory.
//
// Two orthogonal notions of "trust" live in the report and must not be
// conflated:
//   - `CapabilityReport::ready` is a purely *technical* gate: PipeWire, the
//     filter-chain module, and a structurally valid, hash-verified dataset are
//     all present. Only `ready` gates enabling the feature.
//   - `DatasetReport::legal_review_required` is always `true`: passing the
//     technical checks says nothing about whether the HRIR's `license` and
//     `source_url` actually permit use or redistribution. That determination is
//     a human/legal responsibility this code neither makes nor clears. A
//     dataset can therefore be `ready` while its licensing still awaits human
//     review.

/// Manifest schema version. Bumped only on incompatible contract changes.
pub const DATASET_SCHEMA: u8 = 1;

/// The single dataset layout accepted by this MVP: a 14-channel surround HRIR
/// WAV. The token is deliberately vendor-neutral and describes the layout, not
/// any tool or product.
pub const SUPPORTED_DATASET_FORMAT: &str = "surround-7.1-14ch-wav";

/// Exact channel count required in `hrir.wav` (matches the official 7.1 HeSuVi
/// convolver layout: 7 pairs + FC/LFE handling encoded across 14 responses).
pub const REQUIRED_HRIR_CHANNELS: u16 = 14;

/// Sample rates accepted by the MVP. Restricted to the two rates the target
/// hardware and the reference HeSuVi datasets actually use; broaden only with
/// evidence that another rate is needed and verified end to end.
pub const SUPPORTED_SAMPLE_RATES: [u32; 2] = [44_100, 48_000];

const MANIFEST_FILE_NAME: &str = "manifest.json";
const HRIR_FILE_NAME: &str = "hrir.wav";

/// Upper bound for `manifest.json`. It is a tiny metadata file; anything larger
/// is treated as hostile or corrupt rather than parsed.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// Upper bound for `hrir.wav`. A 14-channel HRIR is a short set of impulse
/// responses (a few MB at most); 64 MiB is a generous ceiling that still
/// prevents loading an arbitrarily large file into memory.
const MAX_HRIR_BYTES: u64 = 64 * 1024 * 1024;

/// Substrings that must never appear in a dataset's descriptive metadata, so
/// the product never ships or references proprietary vendor branding. Mirrors
/// the vendor-neutrality guard applied to spatial mode names.
const BANNED_METADATA_SUBSTRINGS: [&str; 4] = ["dts", "dolby", "atmos", "quantum spatial"];

/// User-authored metadata that must sit next to `hrir.wav`. All fields are
/// required; `license` and `source_url` capture provenance for the mandatory
/// human legal review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetManifest {
    pub schema: u8,
    pub format: String,
    pub name: String,
    pub sha256: String,
    pub license: String,
    pub source_url: String,
}

/// Result of validating the dataset directory. Fields are filled in
/// progressively so the preflight/status can report exactly how far validation
/// reached and the first actionable error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetReport {
    pub schema: u8,
    pub manifest_path: Option<String>,
    pub hrir_path: Option<String>,
    pub format: Option<String>,
    pub name: Option<String>,
    pub license: Option<String>,
    pub source_url: Option<String>,
    pub expected_sha256: Option<String>,
    pub observed_sha256: Option<String>,
    pub channels: Option<u16>,
    pub sample_rate: Option<u32>,
    /// Always `true`, independent of `valid`/`ready`. Technical validity (a
    /// well-formed, hash-matched WAV) is orthogonal to legal clearance of the
    /// dataset's `license`/`source_url`, which only a human can determine. This
    /// flag exists so no caller mistakes "passed validation" for "cleared for
    /// use or redistribution".
    pub legal_review_required: bool,
    pub valid: bool,
    pub error: Option<String>,
}

/// The metadata identity used to invalidate the in-memory preflight cache.
/// `dev` and `inode` identify replacement files, while size and nanosecond
/// mtime identify normal in-place updates. Paths remain part of the key so a
/// report never leaks between distinct dataset directories that happen to use
/// hard links to the same files.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DatasetFileFingerprint {
    path: PathBuf,
    device: u64,
    inode: u64,
    size: u64,
    mtime_ns: i128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DatasetCacheKey {
    manifest: DatasetFileFingerprint,
    hrir: DatasetFileFingerprint,
}

#[derive(Debug, Clone)]
struct DatasetValidationCache {
    key: DatasetCacheKey,
    report: DatasetReport,
}

impl DatasetReport {
    fn empty() -> Self {
        Self {
            schema: DATASET_SCHEMA,
            manifest_path: None,
            hrir_path: None,
            format: None,
            name: None,
            license: None,
            source_url: None,
            expected_sha256: None,
            observed_sha256: None,
            channels: None,
            sample_rate: None,
            legal_review_required: true,
            valid: false,
            error: None,
        }
    }
}

/// Standard WAV format tags, plus the extensible sentinel.
const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// Trailing 14 bytes shared by both KSDATAFORMAT_SUBTYPE GUIDs. Only the first
/// two bytes of a SubFormat GUID vary (they mirror the base format tag); the
/// remaining bytes are fixed. Layout: Data1 high half (2), Data2 (2), Data3
/// (2), Data4 (8) = `0000-0010-8000-00aa00389b71`.
const KSDATAFORMAT_SUBTYPE_TAIL: [u8; 14] = [
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];

/// Minimal WAV `fmt`/`data` facts needed to gate the convolver input.
#[derive(Debug, Clone, PartialEq)]
struct WavInfo {
    /// Effective format tag: for `WAVE_FORMAT_EXTENSIBLE` this is the tag
    /// resolved from the SubFormat GUID, not the `0xFFFE` sentinel.
    format_tag: u16,
    channels: u16,
    sample_rate: u32,
    byte_rate: u32,
    block_align: u16,
    bits_per_sample: u16,
    data_bytes: u64,
}

/// Resolves `<dataset_dir>/<file_name>` to a canonical regular-file path,
/// rejecting symlinks and any path that escapes the dataset directory.
fn resolve_regular_file(dataset_dir: &Path, file_name: &str) -> Result<PathBuf, String> {
    let canonical_dir = fs::canonicalize(dataset_dir)
        .map_err(|error| format!("{}: {error}", dataset_dir.display()))?;
    let candidate = canonical_dir.join(file_name);

    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| format!("{}: {error}", candidate.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "{} must be a regular file, not a symbolic link",
            candidate.display()
        ));
    }
    if !metadata.is_file() {
        return Err(format!("{} is not a regular file", candidate.display()));
    }

    // Defense in depth: after resolving, the file must still live directly in
    // the dataset directory. This rejects any link or path that escapes it.
    let resolved = fs::canonicalize(&candidate)
        .map_err(|error| format!("{}: {error}", candidate.display()))?;
    if resolved.parent() != Some(canonical_dir.as_path()) {
        return Err(format!(
            "{} resolves outside the dataset directory",
            candidate.display()
        ));
    }
    Ok(resolved)
}

fn dataset_file_fingerprint(path: PathBuf) -> Result<DatasetFileFingerprint, String> {
    let metadata = fs::metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(DatasetFileFingerprint {
        path,
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
        mtime_ns: i128::from(metadata.mtime()) * 1_000_000_000 + i128::from(metadata.mtime_nsec()),
    })
}

/// Captures the two inputs' metadata before validation. Resolving regular
/// files here preserves the validator's existing no-symlink contract before a
/// cache entry can be considered.
fn dataset_cache_key(dataset_dir: &Path) -> Result<DatasetCacheKey, String> {
    Ok(DatasetCacheKey {
        manifest: dataset_file_fingerprint(resolve_regular_file(dataset_dir, MANIFEST_FILE_NAME)?)?,
        hrir: dataset_file_fingerprint(resolve_regular_file(dataset_dir, HRIR_FILE_NAME)?)?,
    })
}

/// Validates that `source_url` is an HTTP(S) URL with a usable host. The
/// authority (everything after the scheme up to the first `/`, `?`, or `#`) must
/// be non-empty, free of whitespace and control characters, and — after
/// dropping any `userinfo@` prefix and `:port` suffix — must still contain a
/// non-empty host.
fn validate_source_url(source_url: &str) -> Result<(), String> {
    let invalid =
        || "manifest `source_url` must be an http:// or https:// URL with a host".to_string();

    let rest = source_url
        .strip_prefix("https://")
        .or_else(|| source_url.strip_prefix("http://"))
        .ok_or_else(invalid)?;

    // Authority ends at the path/query/fragment delimiter.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return Err(invalid());
    }
    if authority
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(invalid());
    }

    // Drop optional `userinfo@`, then an optional `:port` (leaving bracketed
    // IPv6 literals intact), and require a non-empty host.
    let after_userinfo = authority.rsplit('@').next().unwrap_or("");
    let host = if after_userinfo.starts_with('[') {
        after_userinfo
    } else {
        after_userinfo.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        return Err(invalid());
    }

    Ok(())
}

fn validate_manifest(manifest: &DatasetManifest) -> Result<(), String> {
    if manifest.schema != DATASET_SCHEMA {
        return Err(format!(
            "unsupported manifest schema {}; this build expects {DATASET_SCHEMA}",
            manifest.schema
        ));
    }

    let digest = manifest.sha256.trim().to_ascii_lowercase();
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("manifest `sha256` must be a 64-character hexadecimal SHA-256 digest".into());
    }

    for (field, value) in [
        ("format", &manifest.format),
        ("name", &manifest.name),
        ("license", &manifest.license),
        ("source_url", &manifest.source_url),
    ] {
        if value.trim().is_empty() {
            return Err(format!(
                "manifest field `{field}` is required and must not be empty"
            ));
        }
    }

    if manifest.format.trim() != SUPPORTED_DATASET_FORMAT {
        return Err(format!(
            "manifest `format` must be `{SUPPORTED_DATASET_FORMAT}`; found `{}`",
            manifest.format.trim()
        ));
    }

    // `source_url` documents provenance for the mandatory human legal review, so
    // it must be a fetchable HTTP(S) URL whose authority contains a real host.
    validate_source_url(manifest.source_url.trim())?;

    let haystack = format!("{} {}", manifest.name, manifest.format).to_ascii_lowercase();
    for banned in BANNED_METADATA_SUBSTRINGS {
        if haystack.contains(banned) {
            return Err(format!(
                "manifest metadata must stay vendor-neutral; `{banned}` is not allowed"
            ));
        }
    }

    Ok(())
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes([
        *bytes.get(offset)?,
        *bytes.get(offset + 1)?,
    ]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *bytes.get(offset)?,
        *bytes.get(offset + 1)?,
        *bytes.get(offset + 2)?,
        *bytes.get(offset + 3)?,
    ]))
}

/// Resolves the effective format tag from a `fmt ` chunk body, fully
/// validating a `WAVE_FORMAT_EXTENSIBLE` SubFormat GUID when present.
fn effective_format_tag(bytes: &[u8], body: usize, size: usize) -> Result<u16, String> {
    let base_tag = read_u16_le(bytes, body).ok_or("truncated WAV `fmt ` chunk")?;
    if base_tag != WAVE_FORMAT_EXTENSIBLE {
        return Ok(base_tag);
    }

    // WAVE_FORMAT_EXTENSIBLE: WAVEFORMATEX (16) + cbSize (2) + 22-byte
    // extension (validBits 2, channelMask 4, SubFormat GUID 16).
    if size < 40 {
        return Err("WAV `fmt ` chunk declares WAVE_FORMAT_EXTENSIBLE but is too short".into());
    }
    let cb_size = read_u16_le(bytes, body + 16).ok_or("truncated WAV extension header")?;
    if cb_size < 22 {
        return Err("WAVE_FORMAT_EXTENSIBLE extension size (cbSize) must be at least 22".into());
    }
    // The declared extension must fit inside the `fmt ` chunk: the 16-byte
    // WAVEFORMATEX plus the 2-byte cbSize plus cbSize bytes of extension.
    if 18 + usize::from(cb_size) > size {
        return Err(format!(
            "WAVE_FORMAT_EXTENSIBLE cbSize ({cb_size}) overflows the {size}-byte `fmt ` chunk"
        ));
    }
    let valid_bits = read_u16_le(bytes, body + 18).ok_or("truncated WAV extension header")?;
    let container_bits = read_u16_le(bytes, body + 14).ok_or("truncated WAV `fmt ` chunk")?;
    if valid_bits == 0 || valid_bits > container_bits {
        return Err(format!(
            "WAVE_FORMAT_EXTENSIBLE valid-bits-per-sample ({valid_bits}) must be in 1..={container_bits}"
        ));
    }

    let guid = bytes
        .get(body + 24..body + 40)
        .ok_or("truncated WAVE_FORMAT_EXTENSIBLE SubFormat GUID")?;
    if guid[2..16] != KSDATAFORMAT_SUBTYPE_TAIL {
        return Err(
            "WAVE_FORMAT_EXTENSIBLE SubFormat GUID is not a recognized KSDATAFORMAT_SUBTYPE".into(),
        );
    }
    let sub_tag = u16::from_le_bytes([guid[0], guid[1]]);
    if !matches!(sub_tag, WAVE_FORMAT_PCM | WAVE_FORMAT_IEEE_FLOAT) {
        return Err(format!(
            "WAVE_FORMAT_EXTENSIBLE SubFormat tag {sub_tag} is neither PCM nor IEEE float"
        ));
    }
    Ok(sub_tag)
}

/// Parses just enough of a canonical RIFF/WAVE file to gate the convolver: the
/// `fmt ` chunk (including a full `WAVE_FORMAT_EXTENSIBLE` GUID) and the size
/// of a non-empty `data` chunk. Structural/semantic checks live in
/// [`validate_wav_format`].
fn parse_wav(bytes: &[u8]) -> Result<WavInfo, String> {
    if bytes.len() < 12 {
        return Err("file is too small to be a WAV".into());
    }
    if &bytes[0..4] != b"RIFF" {
        return Err("missing RIFF header; not a WAV file".into());
    }
    if &bytes[8..12] != b"WAVE" {
        return Err("missing WAVE marker; not a WAV file".into());
    }

    let mut offset = 12usize;
    let mut fmt: Option<WavInfo> = None;
    let mut data_bytes: Option<u64> = None;

    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = read_u32_le(bytes, offset + 4).ok_or("truncated chunk header")? as usize;
        let body = offset + 8;
        // Guard the addition itself against overflow on a hostile size field.
        if body.checked_add(size).is_none_or(|end| end > bytes.len()) {
            return Err(format!(
                "chunk `{}` claims {size} bytes but the file is truncated",
                String::from_utf8_lossy(id)
            ));
        }

        if id == b"fmt " {
            if size < 16 {
                return Err("WAV `fmt ` chunk is too small".into());
            }
            fmt = Some(WavInfo {
                format_tag: effective_format_tag(bytes, body, size)?,
                channels: read_u16_le(bytes, body + 2).unwrap(),
                sample_rate: read_u32_le(bytes, body + 4).unwrap(),
                byte_rate: read_u32_le(bytes, body + 8).unwrap(),
                block_align: read_u16_le(bytes, body + 12).unwrap(),
                bits_per_sample: read_u16_le(bytes, body + 14).unwrap(),
                data_bytes: 0,
            });
        } else if id == b"data" {
            data_bytes = Some(size as u64);
        }

        // Chunks are word-aligned: bodies of odd length carry a pad byte.
        offset = body + size + (size & 1);
    }

    let mut info = fmt.ok_or("WAV is missing its `fmt ` chunk")?;
    let data = data_bytes.ok_or("WAV is missing its `data` chunk")?;
    if data == 0 {
        return Err("WAV `data` chunk is empty".into());
    }
    info.data_bytes = data;
    Ok(info)
}

/// Enforces internal consistency of a parsed WAV and that its format/bit-depth
/// combination is one the PipeWire builtin convolver can read.
fn validate_wav_format(info: &WavInfo) -> Result<(), String> {
    if info.channels == 0 {
        return Err("hrir.wav declares zero channels".into());
    }
    if !SUPPORTED_SAMPLE_RATES.contains(&info.sample_rate) {
        return Err(format!(
            "hrir.wav sample rate ({} Hz) is unsupported; this MVP accepts only {:?} Hz",
            info.sample_rate, SUPPORTED_SAMPLE_RATES
        ));
    }
    if info.bits_per_sample == 0 || !info.bits_per_sample.is_multiple_of(8) {
        return Err(format!(
            "hrir.wav bit depth ({}) must be a non-zero multiple of 8",
            info.bits_per_sample
        ));
    }
    let bytes_per_sample = u32::from(info.bits_per_sample / 8);

    // block_align must describe one interleaved frame across all channels.
    let expected_block_align = u32::from(info.channels) * bytes_per_sample;
    if u32::from(info.block_align) != expected_block_align {
        return Err(format!(
            "hrir.wav block align ({}) does not match {} channels * {} bytes/sample ({expected_block_align})",
            info.block_align, info.channels, bytes_per_sample
        ));
    }

    // byte_rate must equal sample_rate * block_align.
    let expected_byte_rate = u64::from(info.sample_rate) * u64::from(info.block_align);
    if u64::from(info.byte_rate) != expected_byte_rate {
        return Err(format!(
            "hrir.wav byte rate ({}) does not match sample rate {} * block align {} ({expected_byte_rate})",
            info.byte_rate, info.sample_rate, info.block_align
        ));
    }

    // data must contain a whole number of frames, and at least one.
    if !info.data_bytes.is_multiple_of(u64::from(info.block_align)) {
        return Err(format!(
            "hrir.wav data length ({} bytes) is not a whole number of {}-byte frames",
            info.data_bytes, info.block_align
        ));
    }
    if info.data_bytes / u64::from(info.block_align) == 0 {
        return Err("hrir.wav contains no audio frames".into());
    }

    match info.format_tag {
        WAVE_FORMAT_PCM if matches!(info.bits_per_sample, 8 | 16 | 24 | 32) => Ok(()),
        WAVE_FORMAT_IEEE_FLOAT if matches!(info.bits_per_sample, 32 | 64) => Ok(()),
        WAVE_FORMAT_PCM | WAVE_FORMAT_IEEE_FLOAT => Err(format!(
            "hrir.wav format tag {} with {}-bit samples is not a valid PCM/float combination",
            info.format_tag, info.bits_per_sample
        )),
        other => Err(format!(
            "hrir.wav uses WAV format tag {other} which the PipeWire convolver cannot read; \
             provide PCM or IEEE-float WAV"
        )),
    }
}

/// Reads a regular file into memory, refusing to load more than `max` bytes.
/// The size is checked from metadata first, then the read itself is capped so a
/// file that grows between the two steps still cannot exceed the bound.
fn read_file_limited(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if metadata.len() > max {
        return Err(format!(
            "{} is {} bytes, exceeding the {max}-byte limit",
            path.display(),
            metadata.len()
        ));
    }

    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    // Read one byte past the limit so an oversized file is detected rather than
    // silently truncated.
    let mut buffer = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut buffer)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if buffer.len() as u64 > max {
        return Err(format!(
            "{} exceeds the {max}-byte limit while reading",
            path.display()
        ));
    }
    Ok(buffer)
}

/// Validates the dataset directory end to end, capturing the first actionable
/// error rather than returning `Err`, so callers always get a full report.
///
/// A successful report is cached only after both inputs have the same metadata
/// identity they had before reading. This makes the common supervisor case
/// cheap without allowing a file rewritten during validation to enter the
/// cache under a stale key.
fn validate_dataset_directory(dataset_dir: &Path) -> DatasetReport {
    validate_dataset_directory_with(dataset_dir, &sha256_hex, &DATASET_VALIDATION_CACHE)
}

/// Internal form with injected dependencies so cache tests remain independent
/// of the process-wide production cache and may run in parallel.
fn validate_dataset_directory_with<F>(
    dataset_dir: &Path,
    hash: &F,
    cache: &Mutex<Option<DatasetValidationCache>>,
) -> DatasetReport
where
    F: Fn(&[u8]) -> String,
{
    let key = dataset_cache_key(dataset_dir).ok();
    if let Some(key) = key.as_ref() {
        let cache = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cached) = cache.as_ref().filter(|cached| &cached.key == key) {
            return cached.report.clone();
        }
    }

    let mut report = DatasetReport::empty();
    match validate_dataset_into(dataset_dir, &mut report, hash) {
        Ok(()) => report.valid = true,
        Err(error) => {
            report.valid = false;
            report.error = Some(error);
        }
    }

    // Do not cache malformed datasets or read errors. A subsequent preflight
    // must retry them so an operator can repair the dataset without restarting
    // the service.
    if report.valid
        && let (Some(before), Ok(after)) = (key, dataset_cache_key(dataset_dir))
        && before == after
    {
        let mut cache = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *cache = Some(DatasetValidationCache {
            key: before,
            report: report.clone(),
        });
    }
    report
}

fn validate_dataset_into<F>(
    dataset_dir: &Path,
    report: &mut DatasetReport,
    hash: &F,
) -> Result<(), String>
where
    F: Fn(&[u8]) -> String,
{
    let manifest_path = resolve_regular_file(dataset_dir, MANIFEST_FILE_NAME)?;
    report.manifest_path = Some(manifest_path.display().to_string());
    let manifest_bytes = read_file_limited(&manifest_path, MAX_MANIFEST_BYTES)?;
    let manifest: DatasetManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
    validate_manifest(&manifest)?;

    let expected = manifest.sha256.trim().to_ascii_lowercase();
    report.format = Some(manifest.format.clone());
    report.name = Some(manifest.name.clone());
    report.license = Some(manifest.license.clone());
    report.source_url = Some(manifest.source_url.clone());
    report.expected_sha256 = Some(expected.clone());

    let hrir_path = resolve_regular_file(dataset_dir, HRIR_FILE_NAME)?;
    report.hrir_path = Some(hrir_path.display().to_string());
    let hrir_bytes = read_file_limited(&hrir_path, MAX_HRIR_BYTES)?;

    let observed = hash(&hrir_bytes);
    report.observed_sha256 = Some(observed.clone());
    if observed != expected {
        return Err(format!(
            "hrir.wav SHA-256 mismatch: manifest expects {expected}, file hashes to {observed}"
        ));
    }

    let wav = parse_wav(&hrir_bytes)?;
    report.channels = Some(wav.channels);
    report.sample_rate = Some(wav.sample_rate);

    if wav.channels != REQUIRED_HRIR_CHANNELS {
        return Err(format!(
            "hrir.wav must have exactly {REQUIRED_HRIR_CHANNELS} channels; found {}",
            wav.channels
        ));
    }
    validate_wav_format(&wav)?;

    Ok(())
}

const SHA256_H0: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

const SHA256_K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// Incremental SHA-256 (FIPS 180-4). Kept in-tree rather than pulling in an
/// external crate because this round of work is constrained to `src/spatial.rs`
/// only (a dependency needs a `Cargo.toml` change). The hasher itself is
/// incremental — data is absorbed a block at a time via [`Sha256::update`] — but
/// note the dataset flow currently hashes the HRIR from a single in-memory
/// buffer (already size-bounded to `MAX_HRIR_BYTES`), not by streaming it from
/// disk. The incremental API keeps a future switch to chunked file reads a
/// local change.
struct Sha256 {
    state: [u32; 8],
    /// Bytes not yet forming a full 64-byte block.
    block: [u8; 64],
    block_len: usize,
    total_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: SHA256_H0,
            block: [0u8; 64],
            block_len: 0,
            total_len: 0,
        }
    }

    fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (index, word) in w.iter_mut().take(16).enumerate() {
            let base = index * 4;
            *word = u32::from_be_bytes([
                block[base],
                block[base + 1],
                block[base + 2],
                block[base + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];

        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);

        // Top off a partially filled block first.
        if self.block_len > 0 {
            let need = 64 - self.block_len;
            let take = need.min(data.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&data[..take]);
            self.block_len += take;
            data = &data[take..];
            if self.block_len < 64 {
                // Still not a full block; keep it buffered for the next call.
                return;
            }
            let block = self.block;
            Self::compress(&mut self.state, &block);
            self.block_len = 0;
        }

        // Consume whole blocks straight from the input.
        let (blocks, remainder) = data.as_chunks::<64>();
        for block in blocks {
            Self::compress(&mut self.state, block);
        }

        // Stash the remainder for the next update/finalize.
        self.block[..remainder.len()].copy_from_slice(remainder);
        self.block_len = remainder.len();
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_length = self.total_len.wrapping_mul(8);

        // Append the 0x80 terminator; block_len is always < 64 here.
        self.block[self.block_len] = 0x80;
        self.block_len += 1;

        if self.block_len > 56 {
            self.block[self.block_len..].fill(0);
            let block = self.block;
            Self::compress(&mut self.state, &block);
            self.block = [0u8; 64];
        } else {
            self.block[self.block_len..56].fill(0);
        }

        self.block[56..64].copy_from_slice(&bit_length.to_be_bytes());
        let block = self.block;
        Self::compress(&mut self.state, &block);

        let mut digest = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Convenience one-shot wrapper over the incremental [`Sha256`].
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_encode(&hasher.finalize())
}

/// Serializes the complete load -> validate -> save transaction.
/// The returned file must remain in scope until the transaction completes.
pub fn mutation_lock() -> Result<File, String> {
    lock_file(MUTATION_LOCK_NAME)
}

/// Holds the wet/dry transition lock until the caller finishes observing,
/// ramping, and verifying the PipeWire controls.  Both the foreground CLI and
/// the systemd supervisor use this lock, so they cannot interleave two ramps
/// and briefly bounce audio to an obsolete gate value.
pub fn mix_lock() -> Result<File, String> {
    lock_file(MIX_LOCK_NAME)
}

fn lock_file(name: &str) -> Result<File, String> {
    #[cfg(test)]
    {
        // Unit tests use synthetic files and command runners.  Do not create
        // persistent locks below the developer's real configuration directory
        // merely to exercise their in-process synchronization contract.
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .map_err(|error| format!("could not open test {name}: {error}"))?;
        lock.lock()
            .map_err(|error| format!("could not lock test {name}: {error}"))?;
        return Ok(lock);
    }

    #[cfg(not(test))]
    {
        let directory = config_directory()?;
        fs::create_dir_all(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
        let path = directory.join(name);
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
    let directory = path.parent().ok_or("invalid spatial configuration path")?;
    let file_name = path
        .file_name()
        .ok_or("invalid spatial configuration path")?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(format!(".tmp-{}-{sequence}", std::process::id()));
    Ok(directory.join(temporary_name))
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    let directory = path.parent().ok_or("invalid spatial configuration path")?;
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

fn corrupt_profile_path(path: &Path) -> Result<PathBuf, String> {
    let directory = path.parent().ok_or("invalid spatial configuration path")?;
    let file_name = path
        .file_name()
        .ok_or("invalid spatial configuration path")?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("could not timestamp corrupt spatial profile: {error}"))?
        .as_nanos();
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut name = file_name.to_os_string();
    name.push(format!(
        ".corrupt-{timestamp}-{}-{sequence}",
        std::process::id()
    ));
    Ok(directory.join(name))
}

/// Preserves an unreadable profile before restoring the safe, disabled default.
/// This is called only while the spatial mutation lock is held.
fn recover_disabled_profile(path: &Path, load_error: &str) -> Result<SpatialProfile, String> {
    let corrupt_path = corrupt_profile_path(path)?;
    fs::rename(path, &corrupt_path).map_err(|error| {
        format!(
            "could not preserve unreadable spatial profile {} as {}: {error}",
            path.display(),
            corrupt_path.display()
        )
    })?;

    let profile = SpatialProfile::default();
    if let Err(error) = save_to_path(&profile, path) {
        // Do not leave the user without their original bytes when the recovery
        // write fails after the successful preservation rename.
        let _ = fs::rename(&corrupt_path, path);
        return Err(format!(
            "could not write disabled replacement for unreadable spatial profile: {error}"
        ));
    }

    eprintln!(
        "spatial disable: unreadable profile preserved at {}; wrote disabled default ({load_error})",
        corrupt_path.display()
    );
    Ok(profile)
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

fn capability_report(
    pipewire_binary_present: bool,
    filter_chain_module_present: bool,
    dataset: DatasetReport,
) -> CapabilityReport {
    let mut missing: Vec<String> = Vec::new();
    if !pipewire_binary_present {
        missing.push("the `pipewire` binary was not found on PATH".into());
    }
    if !filter_chain_module_present {
        missing.push(
            "no PipeWire filter-chain module was found under the known module directories".into(),
        );
    }
    if !dataset.valid {
        missing.push(dataset.error.clone().unwrap_or_else(|| {
            "the open HRTF/binaural dataset under the spatial configuration directory is missing \
             or invalid"
                .into()
        }));
    }
    let hrtf_dataset_present = dataset.valid;
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
        dataset,
        ready,
        error,
    }
}

/// Read-only capability preflight. It inspects `PATH` and well-known
/// filesystem locations only; it never opens a PipeWire connection, executes
/// `pipewire`/`pw-cli`, or touches the default sink, routing, or DSP graph.
pub fn preflight() -> Result<CapabilityReport, String> {
    let dataset = validate_dataset_directory(&hrtf_dataset_directory()?);
    Ok(capability_report(
        binary_present("pipewire"),
        filter_chain_module_present(),
        dataset,
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
/// to report readiness; this records only intent. The persistent lifecycle
/// supervisor observes that intent and owns any later PipeWire work.
pub fn set_enabled(enabled: bool) -> Result<SpatialProfile, String> {
    let _lock = mutation_lock()?;
    let path = config_path()?;
    let profile = match load_from_path(&path) {
        Ok(profile) => profile,
        Err(error) if !enabled => return recover_disabled_profile(&path, &error),
        Err(error) => return Err(error),
    };
    // Disabling is the recovery path.  It must remain available when the
    // dataset, PipeWire installation, or another preflight prerequisite has
    // disappeared, so only an enable request performs the read-only check.
    let next = if enabled {
        let capability = preflight()?;
        enabled_profile(&profile, true, &capability)?
    } else {
        let mut next = profile;
        next.enabled = false;
        validate(next)?
    };
    save_to_path(&next, &path)?;
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

    struct TestConfigDirectory {
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for TestConfigDirectory {
        fn drop(&mut self) {
            *TEST_CONFIG_DIRECTORY
                .lock()
                .expect("clear spatial test configuration directory") = None;
        }
    }

    fn select_test_config_directory(directory: &TemporaryDirectory) -> TestConfigDirectory {
        let serial = TEST_CONFIG_DIRECTORY_SERIAL
            .lock()
            .expect("lock spatial test configuration directory");
        *TEST_CONFIG_DIRECTORY
            .lock()
            .expect("set spatial test configuration directory") = Some(directory.0.clone());
        TestConfigDirectory { _serial: serial }
    }

    fn valid_dataset_report() -> DatasetReport {
        DatasetReport {
            valid: true,
            ..DatasetReport::empty()
        }
    }

    fn invalid_dataset_report() -> DatasetReport {
        DatasetReport::empty()
    }

    /// Builds a well-formed 14-channel PCM WAV with a Dirac impulse in the
    /// first frame of every channel. Synthetic only — it validates channels
    /// and structure, never binaural quality.
    fn synthetic_hrir_wav(channels: u16, sample_rate: u32) -> Vec<u8> {
        const BITS: u16 = 16;
        const FRAMES: usize = 4;
        let block_align = channels * (BITS / 8);
        let data_len = FRAMES * block_align as usize;

        let mut data = vec![0u8; data_len];
        for channel in 0..channels as usize {
            let offset = channel * (BITS / 8) as usize;
            // 0x7FFF: full-scale positive impulse in frame 0.
            data[offset] = 0xFF;
            data[offset + 1] = 0x7F;
        }

        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        let riff_size = 4 + (8 + 16) + (8 + data_len);
        wav.extend_from_slice(&(riff_size as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        let byte_rate = sample_rate * block_align as u32;
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&BITS.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data_len as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        wav
    }

    fn manifest_json(sha256: &str) -> String {
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": DATASET_SCHEMA,
            "format": SUPPORTED_DATASET_FORMAT,
            "name": "Open synthetic surround HRIR",
            "sha256": sha256,
            "license": "CC-BY-4.0",
            "source_url": "https://example.org/open-hrir",
        }))
        .expect("serialize manifest")
    }

    /// Writes an arbitrary `hrir.wav` plus a manifest whose SHA-256 matches it,
    /// and returns that digest.
    fn install_dataset_with_wav(dir: &Path, wav: &[u8]) -> String {
        let digest = sha256_hex(wav);
        fs::write(dir.join(HRIR_FILE_NAME), wav).expect("write hrir.wav");
        fs::write(dir.join(MANIFEST_FILE_NAME), manifest_json(&digest)).expect("write manifest");
        digest
    }

    /// Writes a fully valid dataset (manifest + matching 14-channel WAV) into
    /// `dir` and returns the WAV's SHA-256.
    fn install_valid_dataset(dir: &Path) -> String {
        install_dataset_with_wav(dir, &synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, 48_000))
    }

    fn set_modified_time(path: &Path, offset_seconds: u64) {
        let modified = SystemTime::now()
            .checked_add(std::time::Duration::from_secs(offset_seconds))
            .expect("test timestamp is representable");
        OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open file to set mtime")
            .set_times(fs::FileTimes::new().set_modified(modified))
            .expect("set test file mtime");
    }

    /// Wraps a caller-built `fmt ` chunk body and `data` payload into a RIFF
    /// container, so tests can craft precise (and deliberately malformed) WAVs.
    fn assemble_wav(fmt_body: &[u8], data: &[u8]) -> Vec<u8> {
        let fmt_pad = fmt_body.len() & 1;
        let data_pad = data.len() & 1;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        let riff_size = 4 + (8 + fmt_body.len() + fmt_pad) + (8 + data.len() + data_pad);
        wav.extend_from_slice(&(riff_size as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&(fmt_body.len() as u32).to_le_bytes());
        wav.extend_from_slice(fmt_body);
        if fmt_pad == 1 {
            wav.push(0);
        }
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(data);
        if data_pad == 1 {
            wav.push(0);
        }
        wav
    }

    /// Builds a 16-byte WAVEFORMATEX `fmt ` body with fully explicit fields.
    #[allow(clippy::too_many_arguments)]
    fn pcm_fmt_body(
        format_tag: u16,
        channels: u16,
        sample_rate: u32,
        byte_rate: u32,
        block_align: u16,
        bits: u16,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&format_tag.to_le_bytes());
        body.extend_from_slice(&channels.to_le_bytes());
        body.extend_from_slice(&sample_rate.to_le_bytes());
        body.extend_from_slice(&byte_rate.to_le_bytes());
        body.extend_from_slice(&block_align.to_le_bytes());
        body.extend_from_slice(&bits.to_le_bytes());
        body
    }

    /// Builds a 40-byte WAVE_FORMAT_EXTENSIBLE `fmt ` body with a caller-chosen
    /// SubFormat GUID (first two bytes plus the 14-byte tail).
    fn extensible_fmt_body(
        channels: u16,
        sample_rate: u32,
        bits: u16,
        sub_tag: [u8; 2],
        guid_tail: [u8; 14],
    ) -> Vec<u8> {
        let block_align = channels * (bits / 8);
        let byte_rate = sample_rate * block_align as u32;
        let mut body = pcm_fmt_body(
            WAVE_FORMAT_EXTENSIBLE,
            channels,
            sample_rate,
            byte_rate,
            block_align,
            bits,
        );
        body.extend_from_slice(&22u16.to_le_bytes()); // cbSize
        body.extend_from_slice(&bits.to_le_bytes()); // wValidBitsPerSample
        body.extend_from_slice(&0x0000_063Fu32.to_le_bytes()); // dwChannelMask (7.1)
        body.extend_from_slice(&sub_tag);
        body.extend_from_slice(&guid_tail);
        body
    }

    /// A valid 14-channel WAVE_FORMAT_EXTENSIBLE PCM WAV.
    fn extensible_pcm_wav(channels: u16, sample_rate: u32, bits: u16) -> Vec<u8> {
        let fmt = extensible_fmt_body(
            channels,
            sample_rate,
            bits,
            [WAVE_FORMAT_PCM as u8, 0],
            KSDATAFORMAT_SUBTYPE_TAIL,
        );
        let block_align = channels as usize * (bits / 8) as usize;
        let data = vec![0u8; block_align * 4];
        assemble_wav(&fmt, &data)
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
        let ready = capability_report(true, true, valid_dataset_report());
        assert!(ready.ready);
        assert!(ready.error.is_none());
        assert!(ready.hrtf_dataset_present);

        let missing_pipewire = capability_report(false, true, valid_dataset_report());
        assert!(!missing_pipewire.ready);
        assert!(
            missing_pipewire
                .error
                .as_deref()
                .unwrap()
                .contains("pipewire")
        );

        let missing_module = capability_report(true, false, valid_dataset_report());
        assert!(!missing_module.ready);
        assert!(
            missing_module
                .error
                .as_deref()
                .unwrap()
                .contains("filter-chain")
        );

        let missing_dataset = capability_report(true, true, invalid_dataset_report());
        assert!(!missing_dataset.ready);
        assert!(!missing_dataset.hrtf_dataset_present);
        assert!(missing_dataset.error.as_deref().unwrap().contains("HRTF"));
    }

    #[test]
    fn enabling_without_capability_returns_an_actionable_error_and_does_not_change_the_profile() {
        let profile = SpatialProfile::default();
        let not_ready = capability_report(false, false, invalid_dataset_report());
        let error = enabled_profile(&profile, true, &not_ready)
            .expect_err("enabling without capability must fail");
        assert!(error.contains("spatial capability unavailable"));
        assert!(error.contains("pipewire"));
    }

    #[test]
    fn enabling_with_capability_selects_the_open_binaural_mode() {
        let profile = SpatialProfile::default();
        let ready = capability_report(true, true, valid_dataset_report());
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
        let not_ready = capability_report(false, false, invalid_dataset_report());
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
        assert_eq!(
            load_from_path(&path).expect("default profile"),
            SpatialProfile::default()
        );
    }

    #[test]
    fn disabling_recovers_an_invalid_profile_and_preserves_its_bytes() {
        let directory = TemporaryDirectory::new();
        let _config = select_test_config_directory(&directory);
        let path = directory.profile_path();
        let corrupt = b"{ this is not valid spatial json }";
        fs::write(&path, corrupt).expect("write corrupt profile");

        let profile = set_enabled(false).expect("disable recovers corrupt profile");
        assert_eq!(profile, SpatialProfile::default());
        assert!(!profile.enabled);
        assert_eq!(
            load_from_path(&path).expect("load disabled replacement"),
            SpatialProfile::default()
        );

        let preserved = fs::read_dir(&directory.0)
            .expect("read profile directory")
            .map(|entry| entry.expect("read profile entry").path())
            .find(|entry| {
                entry
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("spatial.json.corrupt-"))
            })
            .expect("preserved corrupt profile");
        assert_eq!(
            fs::read(preserved).expect("read preserved profile"),
            corrupt
        );
    }

    #[test]
    fn enabling_with_an_invalid_profile_fails_closed_without_altering_it() {
        let directory = TemporaryDirectory::new();
        let _config = select_test_config_directory(&directory);
        let path = directory.profile_path();
        let corrupt = b"{ this is not valid spatial json }";
        fs::write(&path, corrupt).expect("write corrupt profile");

        assert!(set_enabled(true).is_err());
        assert_eq!(
            fs::read(&path).expect("read unchanged corrupt profile"),
            corrupt
        );
        assert!(
            fs::read_dir(&directory.0)
                .expect("read profile directory")
                .all(|entry| !entry
                    .expect("read profile entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with("spatial.json.corrupt-"))
        );
    }

    #[test]
    fn sha256_matches_known_answers() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn valid_dataset_validation_reuses_the_cached_hash() {
        let cache = Mutex::new(None);
        let hash_calls = std::cell::Cell::new(0);
        let counting_sha256 = |data: &[u8]| {
            hash_calls.set(hash_calls.get() + 1);
            sha256_hex(data)
        };

        let directory = TemporaryDirectory::new();
        install_valid_dataset(&directory.0);

        let first = validate_dataset_directory_with(&directory.0, &counting_sha256, &cache);
        let second = validate_dataset_directory_with(&directory.0, &counting_sha256, &cache);

        assert!(first.valid, "first validation failed: {:?}", first.error);
        assert!(second.valid, "cached validation failed: {:?}", second.error);
        assert_eq!(
            hash_calls.get(),
            1,
            "unchanged files must not be hashed again"
        );
    }

    #[test]
    fn dataset_cache_invalidates_when_content_size_or_mtime_changes() {
        let cache = Mutex::new(None);
        let hash_calls = std::cell::Cell::new(0);
        let counting_sha256 = |data: &[u8]| {
            hash_calls.set(hash_calls.get() + 1);
            sha256_hex(data)
        };

        let directory = TemporaryDirectory::new();
        let original = synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, 48_000);
        install_dataset_with_wav(&directory.0, &original);
        assert!(validate_dataset_directory_with(&directory.0, &counting_sha256, &cache).valid);

        // Same-size content replacement must be noticed through its mtime.
        let mut changed_contents = original.clone();
        changed_contents[44] ^= 0x01;
        install_dataset_with_wav(&directory.0, &changed_contents);
        set_modified_time(&directory.0.join(HRIR_FILE_NAME), 10);
        assert!(validate_dataset_directory_with(&directory.0, &counting_sha256, &cache).valid);

        // A size change is independently part of the cache key.
        let mut changed_size = changed_contents;
        let extra_frame = usize::from(REQUIRED_HRIR_CHANNELS) * 2;
        changed_size.extend(std::iter::repeat_n(0, extra_frame));
        let data_size = u32::try_from(changed_size.len() - 44).expect("small synthetic WAV");
        changed_size[40..44].copy_from_slice(&data_size.to_le_bytes());
        let riff_size = u32::try_from(changed_size.len() - 8).expect("small synthetic WAV");
        changed_size[4..8].copy_from_slice(&riff_size.to_le_bytes());
        install_dataset_with_wav(&directory.0, &changed_size);
        set_modified_time(&directory.0.join(HRIR_FILE_NAME), 20);
        assert!(validate_dataset_directory_with(&directory.0, &counting_sha256, &cache).valid);

        // Metadata-only changes also require a fresh validation/hash.
        set_modified_time(&directory.0.join(HRIR_FILE_NAME), 30);
        assert!(validate_dataset_directory_with(&directory.0, &counting_sha256, &cache).valid);

        assert_eq!(
            hash_calls.get(),
            4,
            "each changed cache key must trigger a new HRIR hash"
        );
    }

    #[test]
    fn invalid_dataset_and_read_errors_are_not_cached_as_valid() {
        let cache = Mutex::new(None);
        let hash_calls = std::cell::Cell::new(0);
        let counting_sha256 = |data: &[u8]| {
            hash_calls.set(hash_calls.get() + 1);
            sha256_hex(data)
        };

        let directory = TemporaryDirectory::new();
        let wav = synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, 48_000);
        install_dataset_with_wav(&directory.0, &wav);
        assert!(validate_dataset_directory_with(&directory.0, &counting_sha256, &cache).valid);

        // A readable WAV with a stale manifest hash reaches the hashing path,
        // but must never become a cached valid report.
        let mut corrupted = wav.clone();
        corrupted[44] ^= 0x01;
        fs::write(directory.0.join(HRIR_FILE_NAME), &corrupted).expect("corrupt HRIR contents");
        set_modified_time(&directory.0.join(HRIR_FILE_NAME), 10);
        let invalid_first = validate_dataset_directory_with(&directory.0, &counting_sha256, &cache);
        let invalid_second =
            validate_dataset_directory_with(&directory.0, &counting_sha256, &cache);
        assert!(!invalid_first.valid, "bad hash must be rejected");
        assert!(!invalid_second.valid, "invalid report must not be cached");
        assert_eq!(
            hash_calls.get(),
            3,
            "both invalid validations must hash HRIR"
        );

        fs::remove_file(directory.0.join(HRIR_FILE_NAME)).expect("remove HRIR to cause read error");
        let missing = validate_dataset_directory_with(&directory.0, &counting_sha256, &cache);
        assert!(
            !missing.valid,
            "missing HRIR must never reuse a valid report"
        );

        install_dataset_with_wav(&directory.0, &wav);
        set_modified_time(&directory.0.join(HRIR_FILE_NAME), 20);
        let repaired = validate_dataset_directory_with(&directory.0, &counting_sha256, &cache);
        assert!(repaired.valid, "repaired dataset must be validated again");
        assert_eq!(hash_calls.get(), 4);
    }

    #[test]
    fn valid_dataset_is_accepted_and_flags_human_legal_review() {
        let directory = TemporaryDirectory::new();
        let digest = install_valid_dataset(&directory.0);

        let report = validate_dataset_directory(&directory.0);
        assert!(
            report.valid,
            "expected valid dataset, got: {:?}",
            report.error
        );
        assert_eq!(report.channels, Some(REQUIRED_HRIR_CHANNELS));
        assert_eq!(report.sample_rate, Some(48_000));
        assert_eq!(report.expected_sha256.as_deref(), Some(digest.as_str()));
        assert_eq!(report.observed_sha256.as_deref(), Some(digest.as_str()));
        assert_eq!(report.license.as_deref(), Some("CC-BY-4.0"));
        assert_eq!(
            report.source_url.as_deref(),
            Some("https://example.org/open-hrir")
        );
        // License/provenance clearance is never satisfied automatically.
        assert!(report.legal_review_required);
    }

    #[test]
    fn missing_manifest_is_rejected() {
        let directory = TemporaryDirectory::new();
        let wav = synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, 48_000);
        fs::write(directory.0.join(HRIR_FILE_NAME), &wav).expect("write hrir.wav");

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
        assert!(report.error.as_deref().unwrap().contains("manifest.json"));
    }

    #[test]
    fn missing_hrir_file_is_rejected() {
        let directory = TemporaryDirectory::new();
        let wav = synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, 48_000);
        let digest = sha256_hex(&wav);
        fs::write(directory.0.join(MANIFEST_FILE_NAME), manifest_json(&digest))
            .expect("write manifest");

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
        assert!(report.error.as_deref().unwrap().contains("hrir.wav"));
    }

    #[test]
    fn malformed_manifest_json_is_rejected() {
        let directory = TemporaryDirectory::new();
        install_valid_dataset(&directory.0);
        fs::write(directory.0.join(MANIFEST_FILE_NAME), b"{ not json").expect("corrupt manifest");

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
    }

    #[test]
    fn manifest_missing_required_field_is_rejected() {
        let directory = TemporaryDirectory::new();
        let wav = synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, 48_000);
        let digest = sha256_hex(&wav);
        fs::write(directory.0.join(HRIR_FILE_NAME), &wav).expect("write hrir.wav");
        let manifest = serde_json::to_string_pretty(&serde_json::json!({
            "schema": DATASET_SCHEMA,
            "format": SUPPORTED_DATASET_FORMAT,
            "name": "Open synthetic surround HRIR",
            "sha256": digest,
            "license": "   ",
            "source_url": "https://example.org/open-hrir",
        }))
        .expect("serialize manifest");
        fs::write(directory.0.join(MANIFEST_FILE_NAME), manifest).expect("write manifest");

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
        assert!(report.error.as_deref().unwrap().contains("license"));
    }

    #[test]
    fn manifest_vendor_branding_is_rejected() {
        let directory = TemporaryDirectory::new();
        let wav = synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, 48_000);
        let digest = sha256_hex(&wav);
        fs::write(directory.0.join(HRIR_FILE_NAME), &wav).expect("write hrir.wav");
        let manifest = serde_json::to_string_pretty(&serde_json::json!({
            "schema": DATASET_SCHEMA,
            "format": SUPPORTED_DATASET_FORMAT,
            "name": "DTS Headphone X clone",
            "sha256": digest,
            "license": "CC-BY-4.0",
            "source_url": "https://example.org/open-hrir",
        }))
        .expect("serialize manifest");
        fs::write(directory.0.join(MANIFEST_FILE_NAME), manifest).expect("write manifest");

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
        assert!(report.error.as_deref().unwrap().contains("vendor-neutral"));
    }

    #[test]
    fn sha256_mismatch_is_rejected() {
        let directory = TemporaryDirectory::new();
        let wav = synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, 48_000);
        fs::write(directory.0.join(HRIR_FILE_NAME), &wav).expect("write hrir.wav");
        let wrong = "0".repeat(64);
        fs::write(directory.0.join(MANIFEST_FILE_NAME), manifest_json(&wrong))
            .expect("write manifest");

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
        assert!(report.error.as_deref().unwrap().contains("SHA-256"));
        // Both the expected and observed digests are surfaced for diagnosis.
        assert!(report.observed_sha256.is_some());
    }

    #[test]
    fn wrong_channel_count_is_rejected() {
        let directory = TemporaryDirectory::new();
        let wav = synthetic_hrir_wav(8, 48_000);
        let digest = sha256_hex(&wav);
        fs::write(directory.0.join(HRIR_FILE_NAME), &wav).expect("write hrir.wav");
        fs::write(directory.0.join(MANIFEST_FILE_NAME), manifest_json(&digest))
            .expect("write manifest");

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
        assert_eq!(report.channels, Some(8));
        assert!(report.error.as_deref().unwrap().contains("14 channels"));
    }

    #[test]
    fn non_wav_payload_is_rejected() {
        let directory = TemporaryDirectory::new();
        let junk = b"this is definitely not a wav file".to_vec();
        let digest = sha256_hex(&junk);
        fs::write(directory.0.join(HRIR_FILE_NAME), &junk).expect("write hrir.wav");
        fs::write(directory.0.join(MANIFEST_FILE_NAME), manifest_json(&digest))
            .expect("write manifest");

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
        assert!(report.error.as_deref().unwrap().contains("RIFF"));
    }

    #[test]
    fn missing_dataset_directory_reports_actionable_error() {
        let directory = TemporaryDirectory::new();
        let absent = directory.0.join("does-not-exist");
        let report = validate_dataset_directory(&absent);
        assert!(!report.valid);
        assert!(report.error.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_hrir_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = TemporaryDirectory::new();
        let wav = synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, 48_000);
        // Real WAV lives outside the dataset dir; only a symlink points in.
        let outside = directory.0.join("outside.wav");
        fs::write(&outside, &wav).expect("write outside wav");
        let digest = sha256_hex(&wav);
        symlink(&outside, directory.0.join(HRIR_FILE_NAME)).expect("create symlink");
        fs::write(directory.0.join(MANIFEST_FILE_NAME), manifest_json(&digest))
            .expect("write manifest");

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
        assert!(report.error.as_deref().unwrap().contains("symbolic link"));
    }

    #[cfg(unix)]
    #[test]
    fn manifest_path_that_escapes_the_dataset_directory_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = TemporaryDirectory::new();
        let wav = synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, 48_000);
        let digest = sha256_hex(&wav);
        fs::write(directory.0.join(HRIR_FILE_NAME), &wav).expect("write hrir.wav");

        // The manifest is deliberately outside the dataset directory.  A
        // symlink must not turn the fixed manifest name into an escape hatch.
        let outside_manifest = directory.0.join("outside-manifest.json");
        fs::write(&outside_manifest, manifest_json(&digest)).expect("write outside manifest");
        symlink(&outside_manifest, directory.0.join(MANIFEST_FILE_NAME))
            .expect("create manifest symlink");

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
        assert!(report.error.as_deref().unwrap().contains("symbolic link"));
        assert!(report.manifest_path.is_none());
    }

    #[test]
    fn ieee_float_format_tag_is_accepted() {
        // Hand-build a 14-channel IEEE-float (tag 3) WAV and confirm the
        // convolver-readability gate accepts it.
        let channels = REQUIRED_HRIR_CHANNELS;
        let bits: u16 = 32;
        let frames = 2usize;
        let block_align = channels * (bits / 8);
        let data_len = frames * block_align as usize;
        let data = vec![0u8; data_len];

        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        let riff_size = 4 + (8 + 16) + (8 + data_len);
        wav.extend_from_slice(&(riff_size as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        let byte_rate = 48_000u32 * block_align as u32;
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data_len as u32).to_le_bytes());
        wav.extend_from_slice(&data);

        let info = parse_wav(&wav).expect("parse float wav");
        assert_eq!(info.format_tag, 3);
        assert_eq!(info.channels, channels);
        validate_wav_format(&info).expect("float wav is a valid combination");
    }

    #[test]
    fn sha256_multi_block_matches_known_answers() {
        // 56-byte message: padding overflows into a second compression block.
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // One million 'a': thousands of blocks (classic NIST vector).
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            sha256_hex(&million),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn sha256_incremental_matches_one_shot() {
        let data: Vec<u8> = (0..1000u32).map(|value| (value % 251) as u8).collect();
        let one_shot = sha256_hex(&data);
        for chunk in [1usize, 7, 31, 63, 64, 65, 128] {
            let mut hasher = Sha256::new();
            for piece in data.chunks(chunk) {
                hasher.update(piece);
            }
            assert_eq!(
                hex_encode(&hasher.finalize()),
                one_shot,
                "incremental hashing with chunk size {chunk} must match one-shot"
            );
        }
    }

    #[test]
    fn valid_extensible_pcm_dataset_is_accepted() {
        let directory = TemporaryDirectory::new();
        let wav = extensible_pcm_wav(REQUIRED_HRIR_CHANNELS, 48_000, 16);
        install_dataset_with_wav(&directory.0, &wav);

        let report = validate_dataset_directory(&directory.0);
        assert!(
            report.valid,
            "expected valid dataset, got: {:?}",
            report.error
        );
        assert_eq!(report.channels, Some(REQUIRED_HRIR_CHANNELS));
        assert_eq!(report.sample_rate, Some(48_000));
    }

    #[test]
    fn extensible_float_subformat_is_recognized() {
        let channels = REQUIRED_HRIR_CHANNELS;
        let bits = 32u16;
        let fmt = extensible_fmt_body(
            channels,
            48_000,
            bits,
            [WAVE_FORMAT_IEEE_FLOAT as u8, 0],
            KSDATAFORMAT_SUBTYPE_TAIL,
        );
        let data = vec![0u8; channels as usize * (bits / 8) as usize * 2];
        let wav = assemble_wav(&fmt, &data);

        let info = parse_wav(&wav).expect("parse extensible float wav");
        assert_eq!(info.format_tag, WAVE_FORMAT_IEEE_FLOAT);
        validate_wav_format(&info).expect("extensible float is valid");
    }

    #[test]
    fn extensible_with_unknown_subformat_guid_is_rejected() {
        let mut tail = KSDATAFORMAT_SUBTYPE_TAIL;
        tail[13] ^= 0xFF; // corrupt the final GUID byte
        let channels = REQUIRED_HRIR_CHANNELS;
        let bits = 16u16;
        let fmt = extensible_fmt_body(channels, 48_000, bits, [WAVE_FORMAT_PCM as u8, 0], tail);
        let data = vec![0u8; channels as usize * (bits / 8) as usize * 4];
        let wav = assemble_wav(&fmt, &data);

        let error = parse_wav(&wav).expect_err("unknown SubFormat GUID must be rejected");
        assert!(error.contains("SubFormat GUID"));
    }

    #[test]
    fn wav_block_align_mismatch_is_rejected() {
        let channels = REQUIRED_HRIR_CHANNELS;
        let good_block_align = channels * 2;
        let byte_rate = 48_000 * good_block_align as u32;
        let fmt = pcm_fmt_body(
            WAVE_FORMAT_PCM,
            channels,
            48_000,
            byte_rate,
            good_block_align + 2, // wrong
            16,
        );
        let data = vec![0u8; good_block_align as usize * 4];
        let info = parse_wav(&assemble_wav(&fmt, &data)).expect("parse");
        let error = validate_wav_format(&info).expect_err("bad block align");
        assert!(error.contains("block align"));
    }

    #[test]
    fn wav_byte_rate_mismatch_is_rejected() {
        let channels = REQUIRED_HRIR_CHANNELS;
        let block_align = channels * 2;
        let fmt = pcm_fmt_body(
            WAVE_FORMAT_PCM,
            channels,
            48_000,
            48_000 * block_align as u32 + 1, // wrong
            block_align,
            16,
        );
        let data = vec![0u8; block_align as usize * 4];
        let info = parse_wav(&assemble_wav(&fmt, &data)).expect("parse");
        let error = validate_wav_format(&info).expect_err("bad byte rate");
        assert!(error.contains("byte rate"));
    }

    #[test]
    fn wav_partial_frame_data_is_rejected() {
        let channels = REQUIRED_HRIR_CHANNELS;
        let block_align = channels * 2;
        let fmt = pcm_fmt_body(
            WAVE_FORMAT_PCM,
            channels,
            48_000,
            48_000 * block_align as u32,
            block_align,
            16,
        );
        // One byte more than a whole number of frames.
        let data = vec![0u8; block_align as usize + 1];
        let info = parse_wav(&assemble_wav(&fmt, &data)).expect("parse");
        let error = validate_wav_format(&info).expect_err("partial frame");
        assert!(error.contains("whole number"));
    }

    #[test]
    fn wav_invalid_format_bit_depth_combination_is_rejected() {
        let channels = REQUIRED_HRIR_CHANNELS;
        let bits = 16u16; // invalid for IEEE float
        let block_align = channels * (bits / 8);
        let fmt = pcm_fmt_body(
            WAVE_FORMAT_IEEE_FLOAT,
            channels,
            48_000,
            48_000 * block_align as u32,
            block_align,
            bits,
        );
        let data = vec![0u8; block_align as usize * 2];
        let info = parse_wav(&assemble_wav(&fmt, &data)).expect("parse");
        let error = validate_wav_format(&info).expect_err("float must be 32/64-bit");
        assert!(error.contains("PCM/float combination"));
    }

    #[test]
    fn wav_odd_bit_depth_is_rejected() {
        let channels = REQUIRED_HRIR_CHANNELS;
        let fmt = pcm_fmt_body(WAVE_FORMAT_PCM, channels, 48_000, 48_000, 1, 12);
        let data = vec![0u8; 32];
        let info = parse_wav(&assemble_wav(&fmt, &data)).expect("parse");
        let error = validate_wav_format(&info).expect_err("bit depth must be a multiple of 8");
        assert!(error.contains("multiple of 8"));
    }

    #[test]
    fn read_file_limited_enforces_the_bound() {
        let directory = TemporaryDirectory::new();
        let path = directory.0.join("payload.bin");
        fs::write(&path, vec![0u8; 100]).expect("write payload");

        assert!(read_file_limited(&path, 50).is_err());
        assert_eq!(
            read_file_limited(&path, 100)
                .expect("exactly at limit")
                .len(),
            100
        );
        assert_eq!(
            read_file_limited(&path, 200).expect("under limit").len(),
            100
        );
    }

    #[test]
    fn oversized_manifest_is_rejected() {
        let directory = TemporaryDirectory::new();
        install_valid_dataset(&directory.0);
        fs::write(
            directory.0.join(MANIFEST_FILE_NAME),
            vec![b' '; (MAX_MANIFEST_BYTES + 1) as usize],
        )
        .expect("write oversized manifest");

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
        assert!(report.error.as_deref().unwrap().contains("limit"));
    }

    #[test]
    fn extensible_cbsize_overflowing_chunk_is_rejected() {
        let channels = REQUIRED_HRIR_CHANNELS;
        let mut fmt = extensible_fmt_body(
            channels,
            48_000,
            16,
            [WAVE_FORMAT_PCM as u8, 0],
            KSDATAFORMAT_SUBTYPE_TAIL,
        );
        // Claim a 40-byte extension while the chunk only holds 22 bytes of it.
        fmt[16..18].copy_from_slice(&40u16.to_le_bytes());
        let data = vec![0u8; channels as usize * 2 * 4];
        let wav = assemble_wav(&fmt, &data);

        let error = parse_wav(&wav).expect_err("cbSize overflowing the chunk must be rejected");
        assert!(error.contains("cbSize"));
    }

    #[test]
    fn manifest_non_http_source_url_is_rejected() {
        let directory = TemporaryDirectory::new();
        let wav = synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, 48_000);
        let digest = sha256_hex(&wav);
        fs::write(directory.0.join(HRIR_FILE_NAME), &wav).expect("write hrir.wav");
        let manifest = serde_json::to_string_pretty(&serde_json::json!({
            "schema": DATASET_SCHEMA,
            "format": SUPPORTED_DATASET_FORMAT,
            "name": "Open synthetic surround HRIR",
            "sha256": digest,
            "license": "CC-BY-4.0",
            "source_url": "ftp://example.org/hrir",
        }))
        .expect("serialize manifest");
        fs::write(directory.0.join(MANIFEST_FILE_NAME), manifest).expect("write manifest");

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
        assert!(report.error.as_deref().unwrap().contains("source_url"));
    }

    #[test]
    fn source_url_validation_requires_a_real_host() {
        for good in [
            "https://example.org/open-hrir",
            "http://example.org",
            "https://example.org",
            "https://host.example:8443/path?q=1#frag",
            "https://user@host.example/path",
            "https://[2001:db8::1]:8443/path",
        ] {
            assert!(validate_source_url(good).is_ok(), "{good} should be valid");
        }

        for bad in [
            "https://?query",
            "https://#fragment",
            "https:///path",
            "https://   /hrir.wav", // authority is only spaces
            "https:// ",
            "https://\t/x",
            "https://exa mple.org",
            "https://user@", // userinfo but empty host
            "https://:8443/path",
            "ftp://example.org",
            "example.org",
            "",
        ] {
            assert!(
                validate_source_url(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn unsupported_sample_rate_is_rejected() {
        let directory = TemporaryDirectory::new();
        let wav = synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, 96_000);
        install_dataset_with_wav(&directory.0, &wav);

        let report = validate_dataset_directory(&directory.0);
        assert!(!report.valid);
        assert_eq!(report.sample_rate, Some(96_000));
        assert!(report.error.as_deref().unwrap().contains("sample rate"));
    }

    #[test]
    fn supported_sample_rates_are_accepted() {
        for rate in SUPPORTED_SAMPLE_RATES {
            let directory = TemporaryDirectory::new();
            let wav = synthetic_hrir_wav(REQUIRED_HRIR_CHANNELS, rate);
            install_dataset_with_wav(&directory.0, &wav);

            let report = validate_dataset_directory(&directory.0);
            assert!(
                report.valid,
                "{rate} Hz should be valid: {:?}",
                report.error
            );
            assert_eq!(report.sample_rate, Some(rate));
        }
    }
}
