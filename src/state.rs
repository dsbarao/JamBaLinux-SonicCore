use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use zbus::blocking::Connection;

static SIGNAL_CONNECTION: OnceLock<Connection> = OnceLock::new();
static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);
// flock coordinates separate processes. This mutex also makes the same lock
// effective between daemon threads, whose independently opened descriptors do
// not provide a portable in-process locking guarantee.
static STATE_MUTEX: Mutex<()> = Mutex::new(());

// Linux flock(2) constants. SonicCore targets Linux and uses this direct FFI
// to avoid expanding the dependency set for a single system call.
const LOCK_EX: c_int = 2;
const LOCK_UN: c_int = 8;

unsafe extern "C" {
    fn flock(fd: c_int, operation: c_int) -> c_int;
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct RuntimeState {
    pub schema: u8,
    pub updated_at_ms: u128,
    pub headset_connected: Option<bool>,
    pub ambient_mode: Option<String>,
    pub microphone: Option<String>,
    pub lighting_enabled: Option<bool>,
    pub lighting_color: Option<String>,
    pub logo_color: Option<String>,
    pub ring_color: Option<String>,
    pub logo_colors: Option<Vec<String>>,
    pub ring_colors: Option<Vec<String>>,
    pub logo_effect: Option<String>,
    pub ring_effect: Option<String>,
    pub logo_speed: Option<String>,
    pub ring_speed: Option<String>,
    pub battery_percent: Option<u8>,
    pub charging: Option<bool>,
    pub game_chat_value: Option<u8>,
    pub bluetooth: Option<String>,
    pub sidetone_level: Option<String>,
}

/// The cache action to take after sending a complete two-zone lighting
/// profile. `Replace` is used only after every report for that zone was
/// accepted by the kernel; `Unknown` deliberately serializes as JSON `null`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightingZoneCacheUpdate {
    Keep,
    Replace,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightingProfileCacheUpdate {
    Keep,
    Partial {
        logo: LightingZoneCacheUpdate,
        ring: LightingZoneCacheUpdate,
    },
    Complete,
}

/// Decides how a failed complete-profile write affects the runtime cache.
///
/// A profile consists of six reports for Logo, six for Ring, and a final
/// global-enable report. The function intentionally depends only on the
/// report count and the failed report index, so its partial-write behavior is
/// testable without a HID device. An unexpected report layout is handled
/// conservatively by invalidating both zones after any accepted report.
pub fn lighting_profile_cache_update(
    total_reports: usize,
    failed_report_index: Option<usize>,
) -> LightingProfileCacheUpdate {
    const ZONE_REPORTS: usize = 6;
    const COMPLETE_PROFILE_REPORTS: usize = ZONE_REPORTS * 2 + 1;

    match failed_report_index {
        None => LightingProfileCacheUpdate::Complete,
        Some(0) => LightingProfileCacheUpdate::Keep,
        Some(index) if index >= total_reports => LightingProfileCacheUpdate::Keep,
        Some(_) if total_reports != COMPLETE_PROFILE_REPORTS => {
            LightingProfileCacheUpdate::Partial {
                logo: LightingZoneCacheUpdate::Unknown,
                ring: LightingZoneCacheUpdate::Unknown,
            }
        }
        Some(index) if index < ZONE_REPORTS => LightingProfileCacheUpdate::Partial {
            logo: LightingZoneCacheUpdate::Unknown,
            ring: LightingZoneCacheUpdate::Keep,
        },
        Some(index) if index < ZONE_REPORTS * 2 => LightingProfileCacheUpdate::Partial {
            logo: LightingZoneCacheUpdate::Replace,
            ring: LightingZoneCacheUpdate::Unknown,
        },
        // The final global-enable write has no zone selector. If it fails,
        // conservatively invalidate both profiles rather than claiming that
        // the device retained a usable complete profile.
        Some(_) => LightingProfileCacheUpdate::Partial {
            logo: LightingZoneCacheUpdate::Unknown,
            ring: LightingZoneCacheUpdate::Unknown,
        },
    }
}

impl RuntimeState {
    pub fn apply_input(&mut self, report: &[u8]) -> bool {
        let changed = match report {
            [0x02, value @ 0x00..=0x02] => {
                let value = match value {
                    0 => "off",
                    1 => "anc",
                    _ => "talkthru",
                };
                replace_if_changed(&mut self.ambient_mode, value.into())
            }
            [0x03, value @ 0x00..=0x02] => {
                let value = match value {
                    0 => "disconnected",
                    1 => "connected",
                    _ => "pairing",
                };
                replace_if_changed(&mut self.bluetooth, value.into())
            }
            [0x06, value @ 0x00..=0x01] => {
                let value = if *value == 0 { "muted" } else { "active" };
                replace_if_changed(&mut self.microphone, value.into())
            }
            [0x07, value @ 0x00..=0x01] => {
                replace_if_changed(&mut self.lighting_enabled, *value == 1)
            }
            [0x08, value @ 0x00..=0x64] => replace_if_changed(&mut self.battery_percent, *value),
            [0x09, value @ 0x00..=0x01] => {
                replace_if_changed(&mut self.headset_connected, *value == 1)
            }
            [0x10, value @ 0x00..=0x10] => replace_if_changed(&mut self.game_chat_value, *value),
            _ => false,
        };
        if changed {
            self.touch();
        }
        changed
    }

    pub fn touch(&mut self) {
        self.updated_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis());
    }
}

fn replace_if_changed<T: PartialEq>(slot: &mut Option<T>, value: T) -> bool {
    if slot.as_ref() == Some(&value) {
        false
    } else {
        *slot = Some(value);
        true
    }
}

pub fn path() -> Result<PathBuf, String> {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .ok_or("XDG_RUNTIME_DIR is not set; refusing to store runtime state elsewhere")?;
    Ok(PathBuf::from(runtime).join("jambalinux-soniccore-state.json"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum LoadError {
    Missing,
    Corrupt(String),
    Io(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(formatter, "runtime state is absent"),
            Self::Corrupt(error) => write!(formatter, "runtime state is corrupt: {error}"),
            Self::Io(error) => write!(formatter, "failed to read runtime state: {error}"),
        }
    }
}

pub fn load() -> Result<RuntimeState, LoadError> {
    load_at(&path().map_err(LoadError::Io)?)
}

fn load_at(path: &PathBuf) -> Result<RuntimeState, LoadError> {
    let bytes = fs::read(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            LoadError::Missing
        } else {
            LoadError::Io(error.to_string())
        }
    })?;
    serde_json::from_slice(&bytes).map_err(|error| LoadError::Corrupt(error.to_string()))
}

fn lock_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.lock",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state")
    ))
}

struct StateLock<'a> {
    _thread_lock: MutexGuard<'a, ()>,
    file: fs::File,
}

impl StateLock<'static> {
    fn acquire(path: &Path) -> Result<Self, String> {
        let thread_lock = STATE_MUTEX
            .lock()
            .map_err(|_| "runtime-state lock was poisoned".to_owned())?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path(path))
            .map_err(|error| format!("failed to open runtime-state lock: {error}"))?;
        // The lock file lives beside the state file in XDG_RUNTIME_DIR. flock
        // coordinates daemon threads and independent soniccore processes.
        if unsafe { flock(file.as_raw_fd(), LOCK_EX) } != 0 {
            return Err(format!(
                "failed to lock runtime state: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self {
            _thread_lock: thread_lock,
            file,
        })
    }
}

impl Drop for StateLock<'_> {
    fn drop(&mut self) {
        let _ = unsafe { flock(self.file.as_raw_fd(), LOCK_UN) };
    }
}

fn with_exclusive_lock<T>(
    operation: impl FnOnce(&PathBuf) -> Result<T, String>,
) -> Result<T, String> {
    let path = path()?;
    let _lock = StateLock::acquire(&path)?;
    operation(&path)
}

fn temporary_path(path: &Path) -> PathBuf {
    let sequence = TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let thread = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>();
    path.with_file_name(format!(
        ".{}.tmp.{}.{}.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state"),
        std::process::id(),
        thread,
        sequence
    ))
}

fn save_at(path: &PathBuf, state: &mut RuntimeState) -> Result<(), String> {
    state.schema = 1;
    state.touch();
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    let temporary = temporary_path(path);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("failed to create temporary runtime state: {error}"))?;
        file.write_all(&bytes)
            .map_err(|error| format!("failed to write temporary runtime state: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("failed to sync temporary runtime state: {error}"))?;
        fs::rename(&temporary, path)
            .map_err(|error| format!("failed to replace runtime state: {error}"))?;
        fs::File::open(path.parent().unwrap_or_else(|| std::path::Path::new(".")))
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("failed to sync runtime-state directory: {error}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn quarantine_corrupt_at(path: &PathBuf, reason: &str) -> Result<(), String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    let sequence = TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let quarantined = path.with_file_name(format!("{file_name}.corrupt-{timestamp}-{sequence}"));

    fs::rename(path, &quarantined).map_err(|error| {
        format!(
            "failed to quarantine corrupt runtime state at {}: {error}",
            path.display()
        )
    })?;
    eprintln!(
        "quarantined corrupt runtime state at {} to {}: {reason}",
        path.display(),
        quarantined.display()
    );
    Ok(())
}

#[allow(dead_code)]
pub fn save(state: &mut RuntimeState) -> Result<(), String> {
    with_exclusive_lock(|path| save_at(path, state))?;
    emit_changed_signal();
    Ok(())
}

/// Atomically load, modify, and save the runtime state. Missing state starts
/// from defaults; corrupt or unreadable state is never overwritten silently.
fn update_if_inner(
    mutator: impl FnOnce(&mut RuntimeState) -> bool,
    recover_corrupt: bool,
) -> Result<RuntimeState, String> {
    let (state, changed) = with_exclusive_lock(|path| {
        let (mut state, recovered) = match load_at(path) {
            Ok(state) => (state, false),
            Err(LoadError::Missing) => (RuntimeState::default(), false),
            Err(LoadError::Corrupt(reason)) if recover_corrupt => {
                quarantine_corrupt_at(path, &reason)?;
                (RuntimeState::default(), true)
            }
            Err(error) => return Err(error.to_string()),
        };
        let changed = mutator(&mut state) || recovered;
        if changed {
            save_at(path, &mut state)?;
        }
        Ok((state, changed))
    })?;
    if changed {
        emit_changed_signal();
    }
    Ok(state)
}

/// Atomically load and modify the runtime state, persisting and notifying only
/// when the mutator reports a meaningful change. Corrupt state is never
/// overwritten by this command-facing path.
pub fn update_if(mutator: impl FnOnce(&mut RuntimeState) -> bool) -> Result<RuntimeState, String> {
    update_if_inner(mutator, false)
}

/// Daemon-only variant of [`update_if`]. A corrupt cache is preserved beside
/// the state file before a fresh default cache is written, allowing monitoring
/// to resume while command-facing callers continue to report corruption.
pub fn update_recovering_if(
    mutator: impl FnOnce(&mut RuntimeState) -> bool,
) -> Result<RuntimeState, String> {
    update_if_inner(mutator, true)
}

/// Atomically load, modify, and save the runtime state. Missing state starts
/// from defaults; corrupt or unreadable state is never overwritten silently.
pub fn update(mutator: impl FnOnce(&mut RuntimeState)) -> Result<RuntimeState, String> {
    update_if(|state| {
        mutator(state);
        true
    })
}

pub fn init_signal_service() -> Result<(), String> {
    let connection = Connection::session().map_err(|error| error.to_string())?;
    connection
        .request_name("org.jambalinux.soniccore.State")
        .map_err(|error| error.to_string())?;
    SIGNAL_CONNECTION
        .set(connection)
        .map_err(|_| "D-Bus signal service already initialized".to_string())
}

pub fn emit_changed_signal() {
    let Some(connection) = SIGNAL_CONNECTION.get() else {
        return;
    };
    let _ = connection.emit_signal(
        None::<&str>,
        "/org/jambalinux/soniccore/State",
        "org.jambalinux.soniccore.State",
        "Changed",
        &(),
    );
}

pub fn update_control(feature: &str, value: &str) -> Result<(), String> {
    if !matches!(
        feature,
        "ambient" | "lighting" | "color" | "logo-color" | "ring-color" | "sidetone"
    ) {
        return Err(format!("unsupported cached control: {feature}"));
    }
    update(|state| match feature {
        "ambient" => state.ambient_mode = Some(value.into()),
        "lighting" => state.lighting_enabled = Some(value == "on"),
        "color" => {
            state.lighting_enabled = Some(true);
            state.lighting_color = Some(value.into());
            state.logo_color = Some(value.into());
            state.ring_color = Some(value.into());
        }
        "logo-color" => {
            state.lighting_enabled = Some(true);
            state.lighting_color = None;
            state.logo_color = Some(value.into());
        }
        "ring-color" => {
            state.lighting_enabled = Some(true);
            state.lighting_color = None;
            state.ring_color = Some(value.into());
        }
        "sidetone" => state.sidetone_level = Some(value.into()),
        _ => unreachable!("validated above"),
    })
    .map(|_| ())
}

pub fn update_lighting_profile(
    logo_colors: &[String],
    ring_colors: &[String],
    logo_effect: &str,
    ring_effect: &str,
    logo_speed: &str,
    ring_speed: &str,
) -> Result<(), String> {
    update(|state| {
        state.lighting_enabled = Some(true);
        state.logo_colors = Some(logo_colors.to_vec());
        state.ring_colors = Some(ring_colors.to_vec());
        state.logo_color = uniform_color(logo_colors);
        state.ring_color = uniform_color(ring_colors);
        state.lighting_color = if logo_colors == ring_colors {
            uniform_color(logo_colors)
        } else {
            None
        };
        state.logo_effect = Some(logo_effect.into());
        state.ring_effect = Some(ring_effect.into());
        state.logo_speed = Some(logo_speed.into());
        state.ring_speed = Some(ring_speed.into());
    })
    .map(|_| ())
}

/// Persist the portions of a lighting profile that are known after a failed
/// write. This never changes `lighting_enabled`: the final global report was
/// not confirmed, so retaining its previous observed value is safer than
/// inventing a new one.
pub fn update_partial_lighting_profile(
    cache_update: LightingProfileCacheUpdate,
    logo_colors: &[String],
    ring_colors: &[String],
    logo_effect: &str,
    ring_effect: &str,
    logo_speed: &str,
    ring_speed: &str,
) -> Result<(), String> {
    let LightingProfileCacheUpdate::Partial { logo, ring } = cache_update else {
        return Ok(());
    };

    update(|state| {
        apply_zone_cache_update(state, true, logo, logo_colors, logo_effect, logo_speed);
        apply_zone_cache_update(state, false, ring, ring_colors, ring_effect, ring_speed);
        // A partial profile cannot safely claim that both zones share one
        // uniform color, even if the surviving cached values happen to match.
        state.lighting_color = None;
    })
    .map(|_| ())
}

fn apply_zone_cache_update(
    state: &mut RuntimeState,
    logo: bool,
    cache_update: LightingZoneCacheUpdate,
    colors: &[String],
    effect: &str,
    speed: &str,
) {
    match (logo, cache_update) {
        (_, LightingZoneCacheUpdate::Keep) => {}
        (true, LightingZoneCacheUpdate::Replace) => {
            state.logo_colors = Some(colors.to_vec());
            state.logo_color = uniform_color(colors);
            state.logo_effect = Some(effect.into());
            state.logo_speed = Some(speed.into());
        }
        (false, LightingZoneCacheUpdate::Replace) => {
            state.ring_colors = Some(colors.to_vec());
            state.ring_color = uniform_color(colors);
            state.ring_effect = Some(effect.into());
            state.ring_speed = Some(speed.into());
        }
        (true, LightingZoneCacheUpdate::Unknown) => {
            state.logo_colors = None;
            state.logo_color = None;
            state.logo_effect = None;
            state.logo_speed = None;
        }
        (false, LightingZoneCacheUpdate::Unknown) => {
            state.ring_colors = None;
            state.ring_color = None;
            state.ring_effect = None;
            state.ring_speed = None;
        }
    }
}

fn uniform_color(colors: &[String]) -> Option<String> {
    let first = colors.first()?;
    colors
        .iter()
        .all(|color| color == first)
        .then(|| first.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};

    static TEST_ENVIRONMENT: Mutex<()> = Mutex::new(());

    #[test]
    fn first_lighting_report_failure_leaves_cache_intact() {
        assert_eq!(
            lighting_profile_cache_update(13, Some(0)),
            LightingProfileCacheUpdate::Keep
        );
    }

    #[test]
    fn partial_lighting_write_marks_the_affected_zone_unknown() {
        assert_eq!(
            lighting_profile_cache_update(13, Some(3)),
            LightingProfileCacheUpdate::Partial {
                logo: LightingZoneCacheUpdate::Unknown,
                ring: LightingZoneCacheUpdate::Keep,
            }
        );
        assert_eq!(
            lighting_profile_cache_update(13, Some(8)),
            LightingProfileCacheUpdate::Partial {
                logo: LightingZoneCacheUpdate::Replace,
                ring: LightingZoneCacheUpdate::Unknown,
            }
        );
        assert_eq!(
            lighting_profile_cache_update(13, Some(6)),
            LightingProfileCacheUpdate::Partial {
                logo: LightingZoneCacheUpdate::Replace,
                ring: LightingZoneCacheUpdate::Unknown,
            }
        );
        assert_eq!(
            lighting_profile_cache_update(13, Some(12)),
            LightingProfileCacheUpdate::Partial {
                logo: LightingZoneCacheUpdate::Unknown,
                ring: LightingZoneCacheUpdate::Unknown,
            }
        );
        assert_eq!(
            lighting_profile_cache_update(12, Some(3)),
            LightingProfileCacheUpdate::Partial {
                logo: LightingZoneCacheUpdate::Unknown,
                ring: LightingZoneCacheUpdate::Unknown,
            }
        );
    }

    #[test]
    fn complete_lighting_write_records_the_new_profile() {
        assert_eq!(
            lighting_profile_cache_update(13, None),
            LightingProfileCacheUpdate::Complete
        );
    }

    #[test]
    fn partial_lighting_cache_serializes_unknown_zone_as_null() {
        with_test_runtime(|_| {
            let old = vec!["#112233".to_owned(); 5];
            let new = vec!["#445566".to_owned(); 5];
            update_lighting_profile(&old, &old, "solid", "solid", "0.5", "0.5").unwrap();
            update_partial_lighting_profile(
                lighting_profile_cache_update(13, Some(3)),
                &new,
                &new,
                "wave",
                "wave",
                "1.5",
                "1.5",
            )
            .unwrap();

            let cached = load().unwrap();
            assert_eq!(cached.logo_colors, None);
            assert_eq!(cached.logo_effect, None);
            assert_eq!(cached.ring_colors, Some(old));
            let json = serde_json::to_value(cached).unwrap();
            assert!(json["logo_colors"].is_null());
            assert!(json["logo_effect"].is_null());
        });
    }

    fn with_test_runtime<T>(operation: impl FnOnce(&PathBuf) -> T) -> T {
        let _guard = TEST_ENVIRONMENT.lock().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "jambalinux-soniccore-state-test-{}-{}",
            std::process::id(),
            TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let previous = std::env::var_os("XDG_RUNTIME_DIR");
        // The process environment is global, hence the test mutex above.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &directory) };
        let result = operation(&directory);
        match previous {
            Some(value) => unsafe { std::env::set_var("XDG_RUNTIME_DIR", value) },
            None => unsafe { std::env::remove_var("XDG_RUNTIME_DIR") },
        }
        fs::remove_dir_all(&directory).unwrap();
        result
    }

    #[test]
    fn applies_only_confirmed_input_reports() {
        let mut state = RuntimeState::default();
        assert!(state.apply_input(&[0x02, 0x02]));
        assert_eq!(state.ambient_mode.as_deref(), Some("talkthru"));
        assert!(state.apply_input(&[0x07, 0x01]));
        assert_eq!(state.lighting_enabled, Some(true));
        assert!(state.apply_input(&[0x08, 75]));
        assert_eq!(state.battery_percent, Some(75));
        assert!(!state.apply_input(&[0x08, 75]));
        assert!(!state.apply_input(&[0xff, 0x01]));
    }

    #[test]
    fn concurrent_saves_leave_valid_json() {
        with_test_runtime(|_| {
            let workers = (0..8)
                .map(|worker| {
                    std::thread::spawn(move || {
                        for iteration in 0..200 {
                            let mut state = RuntimeState {
                                battery_percent: Some(((worker + iteration) % 101) as u8),
                                ..RuntimeState::default()
                            };
                            save(&mut state).unwrap();
                        }
                    })
                })
                .collect::<Vec<_>>();
            for worker in workers {
                worker.join().unwrap();
            }
            let bytes = fs::read(path().unwrap()).unwrap();
            assert!(serde_json::from_slice::<RuntimeState>(&bytes).is_ok());
        });
    }

    #[test]
    fn concurrent_updates_preserve_all_mutations() {
        with_test_runtime(|_| {
            const WORKERS: usize = 8;
            const UPDATES_PER_WORKER: usize = 50;
            let barrier = Arc::new(Barrier::new(WORKERS));
            let workers = (0..WORKERS)
                .map(|worker| {
                    let barrier = Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        barrier.wait();
                        for iteration in 0..UPDATES_PER_WORKER {
                            update(|state| {
                                state
                                    .logo_colors
                                    .get_or_insert_with(Vec::new)
                                    .push(format!("worker-{worker}-update-{iteration}"));
                            })
                            .unwrap();
                        }
                    })
                })
                .collect::<Vec<_>>();
            for worker in workers {
                worker.join().unwrap();
            }
            let state = load().unwrap();
            assert_eq!(
                state.logo_colors.as_ref().map(Vec::len),
                Some(WORKERS * UPDATES_PER_WORKER)
            );
        });
    }

    #[test]
    fn concurrent_control_updates_preserve_distinct_fields() {
        with_test_runtime(|_| {
            let controls = [
                ("ambient", "anc"),
                ("lighting", "on"),
                ("logo-color", "#112233"),
                ("ring-color", "#445566"),
                ("sidetone", "high"),
            ];
            let barrier = Arc::new(Barrier::new(controls.len()));
            let workers = controls
                .into_iter()
                .map(|(feature, value)| {
                    let barrier = Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        barrier.wait();
                        for _ in 0..50 {
                            update_control(feature, value).unwrap();
                        }
                    })
                })
                .collect::<Vec<_>>();
            for worker in workers {
                worker.join().unwrap();
            }
            let state = load().unwrap();
            assert_eq!(state.ambient_mode.as_deref(), Some("anc"));
            assert_eq!(state.lighting_enabled, Some(true));
            assert_eq!(state.logo_color.as_deref(), Some("#112233"));
            assert_eq!(state.ring_color.as_deref(), Some("#445566"));
            assert_eq!(state.sidetone_level.as_deref(), Some("high"));
        });
    }

    #[test]
    fn corrupt_state_is_reported_and_not_overwritten() {
        with_test_runtime(|_| {
            let state_path = path().unwrap();
            fs::write(&state_path, b"{ not valid json").unwrap();
            assert!(matches!(load(), Err(LoadError::Corrupt(_))));
            let original = fs::read(&state_path).unwrap();
            let error = update_control("sidetone", "high").unwrap_err();
            assert!(error.contains("corrupt"));
            assert_eq!(fs::read(&state_path).unwrap(), original);

            update_recovering_if(|state| {
                state.sidetone_level = Some("high".into());
                true
            })
            .unwrap();
            let quarantined = fs::read_dir(state_path.parent().unwrap())
                .unwrap()
                .map(Result::unwrap)
                .map(|entry| entry.path())
                .find(|candidate| {
                    candidate
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name.starts_with("jambalinux-soniccore-state.json.corrupt-")
                        })
                })
                .expect("corrupt state should be quarantined");
            assert_eq!(fs::read(quarantined).unwrap(), original);
            let recovered = load().unwrap();
            assert_eq!(recovered.sidetone_level.as_deref(), Some("high"));
        });
    }
}
