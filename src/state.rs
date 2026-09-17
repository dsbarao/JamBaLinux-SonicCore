use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use zbus::blocking::Connection;

static SIGNAL_CONNECTION: OnceLock<Connection> = OnceLock::new();

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
            // The headset repeats 0x08 with the same percentage when USB-C
            // power changes. Keep it as a refresh event so charging state is
            // re-read from USB topology even when the battery value is equal.
            [0x08, value @ 0x00..=0x64] => {
                self.battery_percent = Some(*value);
                true
            }
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
    Ok(PathBuf::from(runtime).join("janbalinux-soniccore-state.json"))
}

pub fn load() -> Option<RuntimeState> {
    let path = path().ok()?;
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save(state: &mut RuntimeState) -> Result<(), String> {
    state.schema = 1;
    state.touch();
    let path = path()?;
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
    emit_changed_signal();
    Ok(())
}

pub fn init_signal_service() -> Result<(), String> {
    let connection = Connection::session().map_err(|error| error.to_string())?;
    connection
        .request_name("org.janbalinux.soniccore.State")
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
        "/org/janbalinux/soniccore/State",
        "org.janbalinux.soniccore.State",
        "Changed",
        &(),
    );
}

pub fn update_control(feature: &str, value: &str) -> Result<(), String> {
    let mut state = load().unwrap_or_default();
    match feature {
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
        _ => return Err(format!("unsupported cached control: {feature}")),
    }
    save(&mut state)
}

pub fn update_lighting_profile(
    logo_colors: &[String],
    ring_colors: &[String],
    logo_effect: &str,
    ring_effect: &str,
    logo_speed: &str,
    ring_speed: &str,
) -> Result<(), String> {
    let mut state = load().unwrap_or_default();
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
    save(&mut state)
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

    #[test]
    fn applies_only_confirmed_input_reports() {
        let mut state = RuntimeState::default();
        assert!(state.apply_input(&[0x02, 0x02]));
        assert_eq!(state.ambient_mode.as_deref(), Some("talkthru"));
        assert!(state.apply_input(&[0x07, 0x01]));
        assert_eq!(state.lighting_enabled, Some(true));
        assert!(state.apply_input(&[0x08, 75]));
        assert_eq!(state.battery_percent, Some(75));
        assert!(state.apply_input(&[0x08, 75]));
        assert!(!state.apply_input(&[0xff, 0x01]));
    }
}
