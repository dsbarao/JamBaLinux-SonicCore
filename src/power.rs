//! Battery history and automatic lighting economy.
//!
//! The headset reports its battery percentage on its own (input report
//! `0x08`). The daemon records every change of battery, charging or lighting
//! state in a small JSON-lines history, which turns into a measured drain per
//! hour with the lights on and off and an estimate of the time left.
//!
//! The economy policy only uses the confirmed lighting on/off control: it
//! switches the lights off below a battery threshold away from the charger,
//! or after some minutes without sound, and switches them back on when the
//! condition ends. Switching the lights on by hand during an economy period
//! is respected until the condition arms again. Ambient (ANC/TalkThru) is
//! recorded for analysis only and never changed.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state;

const CONFIG_NAME: &str = "power.json";
const HISTORY_NAME: &str = "battery-history.jsonl";
/// The history is trimmed back to the newest half when it grows past this.
const HISTORY_MAX_LINES: usize = 10_000;
/// Policy and history pass interval.
const TICK: Duration = Duration::from_secs(15);
/// Lights come back after a low-battery economy only this far above the
/// threshold, so they do not flicker around it.
const RESTORE_MARGIN: u8 = 5;
/// A gap this long between two samples means the headset was off or out of
/// range; it is not counted as discharge time.
const MAX_INTERVAL_HOURS: f64 = 3.0;

pub const REASON_LOW_BATTERY: &str = "low-battery";
pub const REASON_IDLE: &str = "idle";

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PowerConfig {
    #[serde(default)]
    pub schema: u8,
    /// Switch the lights off at or below this battery percentage when not
    /// charging. `None` disables it.
    #[serde(default)]
    pub low_battery_percent: Option<u8>,
    /// Switch the lights off after this many minutes without sound on the
    /// headset. `None` disables it.
    #[serde(default)]
    pub idle_minutes: Option<u32>,
}

fn config_path() -> Result<PathBuf, String> {
    Ok(crate::equalizer::config_directory()?.join(CONFIG_NAME))
}

fn history_path() -> Result<PathBuf, String> {
    let root = std::env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".local/state"))
        })
        .ok_or("HOME is not set; refusing to store the battery history elsewhere")?;
    Ok(root.join("jambalinux-soniccore").join(HISTORY_NAME))
}

/// A missing or unreadable configuration means "economy disabled".
pub fn load_config() -> PowerConfig {
    let Ok(path) = config_path() else {
        return PowerConfig::default();
    };
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            eprintln!("{}: {error}; battery economy disabled", path.display());
            PowerConfig::default()
        }),
        Err(_) => PowerConfig::default(),
    }
}

pub fn save_config(config: &PowerConfig) -> Result<(), String> {
    let path = config_path()?;
    let directory = path.parent().ok_or("invalid power configuration path")?;
    fs::create_dir_all(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    let mut config = config.clone();
    config.schema = 1;
    let text = serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?;
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    let written = File::create(&temporary).and_then(|mut file| {
        file.write_all(format!("{text}\n").as_bytes())?;
        file.sync_all()
    });
    if let Err(error) = written.and_then(|()| fs::rename(&temporary, &path)) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("{}: {error}", path.display()));
    }
    Ok(())
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Sample {
    pub at_ms: u128,
    pub battery: u8,
    pub charging: Option<bool>,
    pub lighting: Option<bool>,
    pub effect: Option<String>,
    /// Mean of each LED's brightest channel, 0..1: dim colors may draw less.
    pub brightness: Option<f32>,
    /// Recorded for analysis only; the economy never changes it.
    pub ambient: Option<String>,
    pub saving: Option<String>,
}

fn color_brightness(color: &str) -> Option<f32> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
    let brightest = channel(0..2)?.max(channel(2..4)?).max(channel(4..6)?);
    Some(f32::from(brightest) / 255.0)
}

fn sample_from(cached: &state::RuntimeState, at_ms: u128) -> Option<Sample> {
    let battery = cached.battery_percent?;
    let colors = cached
        .logo_colors
        .iter()
        .chain(cached.ring_colors.iter())
        .flatten()
        .filter_map(|color| color_brightness(color))
        .collect::<Vec<_>>();
    Some(Sample {
        at_ms,
        battery,
        charging: cached.charging,
        lighting: cached.lighting_enabled,
        effect: cached.logo_effect.clone(),
        brightness: (!colors.is_empty()).then(|| colors.iter().sum::<f32>() / colors.len() as f32),
        ambient: cached.ambient_mode.clone(),
        saving: cached.power_saving.clone(),
    })
}

/// Whether two samples differ in what the analysis groups by.
fn significant_change(previous: &Sample, next: &Sample) -> bool {
    previous.battery != next.battery
        || previous.charging != next.charging
        || previous.lighting != next.lighting
}

fn append_sample(sample: &Sample) -> Result<(), String> {
    let path = history_path()?;
    let directory = path.parent().ok_or("invalid battery history path")?;
    fs::create_dir_all(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    let line = serde_json::to_string(sample).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    writeln!(file, "{line}").map_err(|error| format!("{}: {error}", path.display()))?;
    drop(file);
    trim_history(&path)
}

fn trim_history(path: &PathBuf) -> Result<(), String> {
    let lines = read_lines(path);
    if lines.len() <= HISTORY_MAX_LINES {
        return Ok(());
    }
    let keep = &lines[lines.len() - HISTORY_MAX_LINES / 2..];
    let temporary = path.with_extension("jsonl.tmp");
    fs::write(&temporary, format!("{}\n", keep.join("\n")))
        .and_then(|()| fs::rename(&temporary, path))
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn read_lines(path: &PathBuf) -> Vec<String> {
    File::open(path)
        .map(|file| BufReader::new(file).lines().map_while(Result::ok).collect())
        .unwrap_or_default()
}

pub fn read_history() -> Vec<Sample> {
    history_path()
        .map(|path| {
            read_lines(&path)
                .iter()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DrainRate {
    /// Battery percentage points per hour.
    pub percent_per_hour: f64,
    pub hours_measured: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Summary {
    pub samples: usize,
    pub lights_on: Option<DrainRate>,
    pub lights_off: Option<DrainRate>,
    pub recent: Option<DrainRate>,
    /// Estimated hours left at the recent (or overall) drain rate.
    pub hours_left: Option<f64>,
}

/// Rates need at least this much measured discharge to be shown.
const MIN_DROP: f64 = 2.0;
const MIN_HOURS: f64 = 0.5;
/// "Recent" is the discharge measured in the last six hours.
const RECENT_MS: u128 = 6 * 3_600_000;

fn rate(drop: f64, hours: f64) -> Option<DrainRate> {
    (drop >= MIN_DROP && hours >= MIN_HOURS).then(|| DrainRate {
        percent_per_hour: drop / hours,
        hours_measured: hours,
    })
}

/// Measured drain from consecutive discharging samples. An interval counts
/// only when both ends are off the charger, the battery did not rise, and the
/// gap is short enough that the headset was in use.
pub fn summarize(samples: &[Sample], now: u128) -> Summary {
    let mut totals = [(0.0_f64, 0.0_f64); 3]; // lights on, lights off, recent
    for pair in samples.windows(2) {
        let (start, end) = (&pair[0], &pair[1]);
        if start.charging != Some(false)
            || end.charging != Some(false)
            || end.battery > start.battery
        {
            continue;
        }
        let hours = end.at_ms.saturating_sub(start.at_ms) as f64 / 3_600_000.0;
        if hours <= 0.0 || hours > MAX_INTERVAL_HOURS {
            continue;
        }
        let drop = f64::from(start.battery - end.battery);
        let group = if start.lighting == Some(true) { 0 } else { 1 };
        totals[group].0 += drop;
        totals[group].1 += hours;
        if now.saturating_sub(end.at_ms) <= RECENT_MS {
            totals[2].0 += drop;
            totals[2].1 += hours;
        }
    }
    let lights_on = rate(totals[0].0, totals[0].1);
    let lights_off = rate(totals[1].0, totals[1].1);
    let recent = rate(totals[2].0, totals[2].1);
    let overall = rate(totals[0].0 + totals[1].0, totals[0].1 + totals[1].1);
    let latest = samples.last();
    let hours_left = latest
        .filter(|sample| sample.charging == Some(false))
        .and_then(|sample| {
            recent
                .as_ref()
                .or(overall.as_ref())
                .map(|rate| f64::from(sample.battery) / rate.percent_per_hour)
        });
    Summary {
        samples: samples.len(),
        lights_on,
        lights_off,
        recent,
        hours_left,
    }
}

/// What the economy should do on this pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Keep,
    /// Switch the lights off and remember why.
    TurnOff(&'static str),
    /// Switch the lights back on; the economy period is over.
    Restore,
    /// Someone switched the lights on during the economy: stop tracking it
    /// and do not switch them off again until the condition re-arms.
    UserOverride,
    /// Lights are off for another reason now; keep them off but change it.
    Retag(&'static str),
}

#[derive(Clone, Debug, Default)]
pub struct Inputs {
    pub battery: Option<u8>,
    pub charging: Option<bool>,
    pub lighting: Option<bool>,
    pub headset_connected: Option<bool>,
    pub idle_seconds: u64,
    pub saving: Option<String>,
    pub suppress_low: bool,
    pub suppress_idle: bool,
}

pub fn decide(config: &PowerConfig, inputs: &Inputs) -> Action {
    let low = config.low_battery_percent.is_some_and(|threshold| {
        inputs.charging == Some(false) && inputs.battery.is_some_and(|battery| battery <= threshold)
    });
    let idle = config.idle_minutes.is_some_and(|minutes| {
        inputs.headset_connected != Some(false) && inputs.idle_seconds >= u64::from(minutes) * 60
    });

    match inputs.saving.as_deref() {
        None => {
            if inputs.lighting != Some(true) {
                Action::Keep
            } else if low && !inputs.suppress_low {
                Action::TurnOff(REASON_LOW_BATTERY)
            } else if idle && !inputs.suppress_idle {
                Action::TurnOff(REASON_IDLE)
            } else {
                Action::Keep
            }
        }
        Some(_) if inputs.lighting == Some(true) => Action::UserOverride,
        Some(REASON_LOW_BATTERY) => {
            let recovered = inputs.charging == Some(true)
                || config.low_battery_percent.is_none_or(|threshold| {
                    inputs
                        .battery
                        .is_some_and(|battery| battery > threshold.saturating_add(RESTORE_MARGIN))
                });
            if !recovered {
                Action::Keep
            } else if idle {
                Action::Retag(REASON_IDLE)
            } else {
                Action::Restore
            }
        }
        Some(_) => {
            if idle {
                Action::Keep
            } else if low {
                Action::Retag(REASON_LOW_BATTERY)
            } else {
                Action::Restore
            }
        }
    }
}

/// Whether any of the Quantum's PCM nodes is processing audio right now.
fn headset_audio_active() -> Option<bool> {
    let objects = crate::pipewire::pw_dump().ok()?;
    Some(objects.iter().any(|object| {
        object.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Node")
            && object.get("info").is_some_and(|info| {
                info.get("state").and_then(Value::as_str) == Some("running")
                    && info
                        .get("props")
                        .and_then(|props| props.get("alsa.components"))
                        .and_then(Value::as_str)
                        == Some("USB0ecb:2069")
            })
    }))
}

fn switch_lighting(on: bool) -> Result<(), String> {
    let command = crate::parse_set_command(
        [
            "lighting".to_owned(),
            if on { "on" } else { "off" }.to_owned(),
        ]
        .into_iter(),
    )?;
    match crate::set_control(command) {
        Ok(true) => Ok(()),
        Ok(false) => Err("the JBL Quantum 810 dongle is not connected".into()),
        Err(error) => Err(error),
    }
}

fn set_saving(reason: Option<&str>) -> Result<(), String> {
    state::update_recovering_if(|cached| {
        let next = reason.map(str::to_owned);
        let changed = cached.power_saving != next;
        cached.power_saving = next;
        changed
    })
    .map(|_| ())
}

/// Daemon thread: records the history and applies the economy.
pub fn spawn_policy() -> Result<(), String> {
    thread::Builder::new()
        .name("soniccore-power".into())
        .spawn(|| {
            let mut last_sample: Option<Sample> = read_history().pop();
            let mut last_audio = Instant::now();
            let mut suppress_low = false;
            let mut suppress_idle = false;
            loop {
                thread::sleep(TICK);
                let Ok(cached) = state::load() else {
                    continue;
                };

                if let Some(sample) = sample_from(&cached, now_ms())
                    && last_sample
                        .as_ref()
                        .is_none_or(|previous| significant_change(previous, &sample))
                {
                    if let Err(error) = append_sample(&sample) {
                        eprintln!("battery history: {error}");
                    }
                    last_sample = Some(sample);
                }

                let config = load_config();
                // Only spend a pw-dump when the idle economy is enabled.
                if config.idle_minutes.is_some() {
                    if headset_audio_active() != Some(false) {
                        last_audio = Instant::now();
                        suppress_idle = false;
                    }
                } else {
                    last_audio = Instant::now();
                }
                if cached.charging == Some(true) {
                    suppress_low = false;
                }

                let action = decide(
                    &config,
                    &Inputs {
                        battery: cached.battery_percent,
                        charging: cached.charging,
                        lighting: cached.lighting_enabled,
                        headset_connected: cached.headset_connected,
                        idle_seconds: last_audio.elapsed().as_secs(),
                        saving: cached.power_saving.clone(),
                        suppress_low,
                        suppress_idle,
                    },
                );
                let result = match action {
                    Action::Keep => Ok(()),
                    Action::TurnOff(reason) => {
                        // Recorded first, so the refusal of OpenRGB writes
                        // covers the switch-off itself.
                        set_saving(Some(reason)).and_then(|()| {
                            switch_lighting(false).inspect_err(|_| {
                                let _ = set_saving(None);
                            })
                        })
                    }
                    Action::Restore => set_saving(None).and_then(|()| switch_lighting(true)),
                    Action::Retag(reason) => set_saving(Some(reason)),
                    Action::UserOverride => {
                        match cached.power_saving.as_deref() {
                            Some(REASON_LOW_BATTERY) => suppress_low = true,
                            _ => suppress_idle = true,
                        }
                        set_saving(None)
                    }
                };
                if let Err(error) = result {
                    eprintln!("battery economy: {error}");
                }
            }
        })
        .map(|_| ())
        .map_err(|error| format!("failed to start the battery economy: {error}"))
}

#[derive(Serialize)]
struct PowerOutput {
    schema: u8,
    config: PowerConfig,
    saving: Option<String>,
    history: Summary,
}

/// `soniccore power [--format json]`
pub fn print_status(json: bool) -> Result<(), String> {
    let config = load_config();
    let saving = state::load().ok().and_then(|cached| cached.power_saving);
    let history = summarize(&read_history(), now_ms());
    if json {
        let output = PowerOutput {
            schema: 1,
            config,
            saving,
            history,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&output).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    let describe = |value: Option<String>| value.unwrap_or_else(|| "off".into());
    println!(
        "low battery: {}",
        describe(
            config
                .low_battery_percent
                .map(|value| format!("lights off at {value}%"))
        )
    );
    println!(
        "idle: {}",
        describe(
            config
                .idle_minutes
                .map(|value| format!("lights off after {value} min without sound"))
        )
    );
    println!("saving now: {}", describe(saving));
    let show = |label: &str, rate: &Option<DrainRate>| match rate {
        Some(rate) => println!(
            "{label}: {:.1}%/h over {:.1} h",
            rate.percent_per_hour, rate.hours_measured
        ),
        None => println!("{label}: not enough data yet"),
    };
    show("drain, lights on", &history.lights_on);
    show("drain, lights off", &history.lights_off);
    match history.hours_left {
        Some(hours) => println!("estimated time left: {hours:.1} h"),
        None => println!("estimated time left: not enough data yet"),
    }
    Ok(())
}

/// `soniccore power set <low-battery|idle> <off|N>`
pub fn set(setting: &str, value: &str) -> Result<(), String> {
    let mut config = load_config();
    let number = |range: std::ops::RangeInclusive<u32>| -> Result<Option<u32>, String> {
        if value == "off" {
            return Ok(None);
        }
        value
            .parse::<u32>()
            .ok()
            .filter(|number| range.contains(number))
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "{setting} must be off or {}..={}",
                    range.start(),
                    range.end()
                )
            })
    };
    match setting {
        "low-battery" => config.low_battery_percent = number(5..=90)?.map(|value| value as u8),
        "idle" => config.idle_minutes = number(1..=120)?,
        _ => return Err("usage: soniccore power set <low-battery|idle> <off|N>".into()),
    }
    save_config(&config)?;
    println!("{setting} set to {value}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(minutes: u128, battery: u8, charging: bool, lighting: bool) -> Sample {
        Sample {
            at_ms: minutes * 60_000,
            battery,
            charging: Some(charging),
            lighting: Some(lighting),
            effect: Some("solid".into()),
            brightness: Some(1.0),
            ambient: Some("anc".into()),
            saving: None,
        }
    }

    #[test]
    fn drain_is_measured_separately_with_lights_on_and_off() {
        let samples = vec![
            // Lights on: 6 points in 60 minutes.
            sample(0, 80, false, true),
            sample(20, 78, false, true),
            sample(40, 76, false, true),
            sample(60, 74, false, true),
            // Lights off: 3 points in 60 minutes.
            sample(60, 74, false, false),
            sample(80, 73, false, false),
            sample(100, 72, false, false),
            sample(120, 71, false, false),
        ];
        let summary = summarize(&samples, 120 * 60_000);
        assert!((summary.lights_on.as_ref().unwrap().percent_per_hour - 6.0).abs() < 0.01);
        assert!((summary.lights_off.as_ref().unwrap().percent_per_hour - 3.0).abs() < 0.01);
        // 9 points over 2 hours recently; 71% left.
        assert!((summary.hours_left.unwrap() - 71.0 / 4.5).abs() < 0.01);
    }

    #[test]
    fn charging_long_gaps_and_too_little_data_are_ignored() {
        let samples = vec![
            sample(0, 50, false, true),
            sample(20, 49, true, true),   // charging: ignored
            sample(400, 60, false, true), // 6 h gap: ignored
            sample(420, 59, false, true),
        ];
        let summary = summarize(&samples, 420 * 60_000);
        assert_eq!(summary.lights_on, None, "1 point in 20 min is not enough");
        assert_eq!(summary.hours_left, None);
        assert_eq!(summarize(&[], 0), Summary::default());
    }

    fn config(low: Option<u8>, idle: Option<u32>) -> PowerConfig {
        PowerConfig {
            schema: 1,
            low_battery_percent: low,
            idle_minutes: idle,
        }
    }

    fn inputs(battery: u8, charging: bool, lighting: bool, idle_seconds: u64) -> Inputs {
        Inputs {
            battery: Some(battery),
            charging: Some(charging),
            lighting: Some(lighting),
            headset_connected: Some(true),
            idle_seconds,
            ..Inputs::default()
        }
    }

    #[test]
    fn low_battery_switches_off_and_back_with_a_margin() {
        let config = config(Some(20), None);
        assert_eq!(decide(&config, &inputs(21, false, true, 0)), Action::Keep);
        assert_eq!(
            decide(&config, &inputs(20, false, true, 0)),
            Action::TurnOff(REASON_LOW_BATTERY)
        );
        // On the charger the lights stay as they are.
        assert_eq!(decide(&config, &inputs(10, true, true, 0)), Action::Keep);

        let saving = |battery, charging| Inputs {
            saving: Some(REASON_LOW_BATTERY.into()),
            ..inputs(battery, charging, false, 0)
        };
        assert_eq!(
            decide(&config, &saving(24, false)),
            Action::Keep,
            "inside the margin"
        );
        assert_eq!(decide(&config, &saving(26, false)), Action::Restore);
        assert_eq!(
            decide(&config, &saving(15, true)),
            Action::Restore,
            "charging"
        );
    }

    #[test]
    fn idle_switches_off_after_the_minutes_and_returns_with_sound() {
        let config = config(None, Some(10));
        assert_eq!(decide(&config, &inputs(80, false, true, 599)), Action::Keep);
        assert_eq!(
            decide(&config, &inputs(80, false, true, 600)),
            Action::TurnOff(REASON_IDLE)
        );
        let saving = Inputs {
            saving: Some(REASON_IDLE.into()),
            ..inputs(80, false, false, 0)
        };
        assert_eq!(decide(&config, &saving), Action::Restore);
        // Headset off: no idle economy.
        let off = Inputs {
            headset_connected: Some(false),
            ..inputs(80, false, true, 6000)
        };
        assert_eq!(decide(&config, &off), Action::Keep);
    }

    #[test]
    fn manual_choices_are_respected() {
        let config = config(Some(20), Some(10));
        // Lights already off by the user: nothing to do.
        assert_eq!(
            decide(&config, &inputs(10, false, false, 9999)),
            Action::Keep
        );
        // Switched back on during the economy: tracked as an override.
        let during = Inputs {
            saving: Some(REASON_IDLE.into()),
            ..inputs(80, false, true, 9999)
        };
        assert_eq!(decide(&config, &during), Action::UserOverride);
        // While suppressed, the same condition does not switch off again.
        let suppressed = Inputs {
            suppress_low: true,
            ..inputs(10, false, true, 0)
        };
        assert_eq!(decide(&config, &suppressed), Action::Keep);
    }

    #[test]
    fn an_ending_condition_hands_over_to_one_still_active() {
        let config = config(Some(20), Some(10));
        let idle_then_low = Inputs {
            saving: Some(REASON_IDLE.into()),
            ..inputs(10, false, false, 0)
        };
        assert_eq!(
            decide(&config, &idle_then_low),
            Action::Retag(REASON_LOW_BATTERY)
        );
        let low_then_idle = Inputs {
            saving: Some(REASON_LOW_BATTERY.into()),
            ..inputs(80, true, false, 9999)
        };
        assert_eq!(decide(&config, &low_then_idle), Action::Retag(REASON_IDLE));
    }

    #[test]
    fn brightness_reads_the_brightest_channel() {
        assert_eq!(color_brightness("#ff0000"), Some(1.0));
        assert_eq!(color_brightness("#000000"), Some(0.0));
        assert!((color_brightness("#800000").unwrap() - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(color_brightness("red"), None);
    }
}
