//! "Luzes do PC": Sonic Core in charge of every light of the computer.
//!
//! OpenRGB stays the driver of the devices (memory, motherboard, video
//! card, keyboards...) and Sonic Core decides their colors through OpenRGB's
//! SDK server ([`crate::openrgb_sdk`]): a theme (a palette), an effect
//! drawn here frame by frame (some follow the music through the spectrum
//! meter), automatic themes while a given application or any game plays,
//! and short flashes for the headset's events (microphone muted, low
//! battery). The headset follows the same theme with its own built-in
//! effects, through the same allowlisted commands as `soniccore set`.
//!
//! `soniccore lights run` is the engine (a user service); the other
//! `soniccore lights ...` commands change `pc-lights.json`, which the engine
//! follows within a fraction of a second, and report its state.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::f32::consts::TAU;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::openrgb_sdk::{self, Client, Color, Controller};

const CONFIG_FILE: &str = "pc-lights.json";
/// What OpenRGB showed before Sonic Core took the lights (each device's
/// mode and colors, the Effects plugin's running effects), put back when
/// OpenRGB takes them again; kept on disk across restarts.
const HANDOVER_FILE: &str = "pc-lights-handover.json";
/// Frames kept per device to tell our colors from someone else's.
const RECENT_FRAMES: usize = 8;
const STATUS_FILE: &str = "jambalinux-soniccore-pc-lights.json";
const LIVE_FILE: &str = "jambalinux-soniccore-pc-lights-live.json";
/// A profile the user picked in the app: the engine hands the lights to
/// OpenRGB with it (one writer, in one order).
const REQUEST_FILE: &str = "jambalinux-soniccore-pc-lights-request.json";
/// The colors being drawn, for the app's own effects to follow (the same
/// shape as the OpenRGB plugin's file: ten colors, logo then ring).
const FRAME_FILE: &str = "jambalinux-soniccore-pc-lights-frame.json";
const FRAME_PUBLISH: Duration = Duration::from_millis(100);
/// The OpenRGB Effects plugin runs our SDK commands on the server's network
/// thread, beside its own effect threads: starting or stopping effects, or
/// loading a profile, right after another such change can crash OpenRGB
/// (seen with a profile loaded just after an effect started). Changes are
/// kept this far apart.
const EFFECTS_SETTLE: Duration = Duration::from_secs(2);
/// After (re)connecting, OpenRGB may still be starting its effects.
const CONNECT_SETTLE: Duration = Duration::from_secs(5);
const CLIENT_NAME: &str = "JamBaLinux Sonic Core";
/// Our own plugin's virtual headset in OpenRGB: the headset is driven
/// directly, never through OpenRGB.
const OWN_DEVICE_LOCATION: &str = "soniccore:";
const FRAME: Duration = Duration::from_millis(33);
const IDLE: Duration = Duration::from_millis(250);
const RETRY: Duration = Duration::from_secs(3);
/// The status file is older than this: the engine is not running.
const STALE_MS: u128 = 5_000;
const MAX_RULES: usize = 50;
const MAX_LIVE_COLORS: usize = 6;

pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub colors: &'static [&'static str],
}

/// Fixed palettes. The open-source color schemes keep their names (MIT
/// licensed palettes, credited in the README); the others are our own.
pub const THEMES: &[Theme] = &[
    Theme { id: "synthwave", name: "Synthwave", colors: &["#ff2a6d", "#d300c5", "#7b2cff", "#05d9e8"] },
    Theme { id: "aurora", name: "Aurora", colors: &["#00ffa3", "#00d1ff", "#7a5cff", "#2affd5"] },
    Theme { id: "oceano", name: "Oceano", colors: &["#003cff", "#0096ff", "#00e5ff", "#00ffd0"] },
    Theme { id: "lava", name: "Lava", colors: &["#ff2000", "#ff5a00", "#ff9100", "#ffd000"] },
    Theme { id: "por-do-sol", name: "Pôr do sol", colors: &["#ff3c00", "#ff007a", "#ff9a00", "#a100ff"] },
    Theme { id: "floresta", name: "Floresta", colors: &["#00ff44", "#6aff00", "#00ff9d", "#b4ff00"] },
    Theme { id: "gelo", name: "Gelo", colors: &["#ffffff", "#9ee7ff", "#4fc3ff", "#c9f6ff"] },
    Theme { id: "matrix", name: "Matrix", colors: &["#00ff41", "#00c832", "#39ff14", "#008f11"] },
    Theme { id: "catppuccin", name: "Catppuccin", colors: &["#cba6f7", "#f5c2e7", "#89dceb", "#94e2d5", "#fab387"] },
    Theme { id: "dracula", name: "Dracula", colors: &["#bd93f9", "#ff79c6", "#8be9fd", "#50fa7b", "#ffb86c"] },
    Theme { id: "nord", name: "Nord", colors: &["#88c0d0", "#81a1c1", "#5e81ac", "#8fbcbb", "#b48ead"] },
    Theme { id: "tokyo-night", name: "Tokyo Night", colors: &["#7aa2f7", "#bb9af7", "#7dcfff", "#9ece6a", "#ff9e64"] },
    Theme { id: "gruvbox", name: "Gruvbox", colors: &["#fb4934", "#fabd2f", "#b8bb26", "#83a598", "#d3869b"] },
    Theme { id: "vermelho", name: "Vermelho", colors: &["#ff0000"] },
    Theme { id: "branco", name: "Branco", colors: &["#ffffff"] },
];

/// Palettes that change: the headset's own colors, the desktop wallpaper's
/// and the cover of what is playing (these two sent by the app).
pub const LIVE_THEMES: &[(&str, &str)] = &[
    ("headset", "Cores do fone"),
    ("wallpaper", "Papel de parede"),
    ("cover", "Capa do que toca"),
];

pub const EFFECTS: &[(&str, &str)] = &[
    ("solid", "Sólido"),
    ("breathing", "Respiração"),
    ("wave", "Onda"),
    ("rainbow", "Arco-íris"),
    ("stars", "Estrelas"),
    ("music", "Música"),
    ("vu", "VU"),
];

fn theme_known(id: &str) -> bool {
    THEMES.iter().any(|theme| theme.id == id) || LIVE_THEMES.iter().any(|(live, _)| *live == id)
}

fn effect_known(id: &str) -> bool {
    EFFECTS.iter().any(|(effect, _)| *effect == id)
}

fn effect_needs_sound(id: &str) -> bool {
    matches!(id, "music" | "vu")
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Look {
    pub theme: String,
    pub effect: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub app: String,
    pub theme: String,
    pub effect: String,
}

fn default_theme() -> String {
    "synthwave".into()
}
fn default_effect() -> String {
    "wave".into()
}
fn default_speed() -> f32 {
    1.0
}
fn default_brightness() -> u8 {
    100
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub schema: u8,
    /// Sonic Core in charge of the lights (false: OpenRGB is).
    #[serde(default)]
    pub active: bool,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_effect")]
    pub effect: String,
    /// 0.25 (slow) to 3 (fast).
    #[serde(default = "default_speed")]
    pub speed: f32,
    /// 5 to 100 percent.
    #[serde(default = "default_brightness")]
    pub brightness: u8,
    /// The headset follows the theme too.
    #[serde(default = "yes")]
    pub headset: bool,
    /// Flashes for the microphone and a low battery.
    #[serde(default = "yes")]
    pub alerts: bool,
    /// Devices left to OpenRGB, by [`Controller::key`].
    #[serde(default)]
    pub excluded: BTreeSet<String>,
    /// A look while an application plays (first match wins).
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// A look while any game plays.
    #[serde(default)]
    pub games: Option<Look>,
    /// The OpenRGB profile loaded when OpenRGB takes the lights back.
    #[serde(default)]
    pub restore_profile: Option<String>,
    /// The SDK server's port when it is not OpenRGB's own setting.
    #[serde(default)]
    pub port: Option<u16>,
}

impl Default for Config {
    fn default() -> Self {
        serde_json::from_str("{}").expect("defaults")
    }
}

fn config_path() -> Result<PathBuf, String> {
    Ok(crate::equalizer::config_directory()?.join(CONFIG_FILE))
}

fn runtime_path(name: &str) -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .ok_or("XDG_RUNTIME_DIR is not set")?;
    Ok(PathBuf::from(runtime).join(name))
}

fn write_atomic(path: &PathBuf, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    let written = File::create(&temporary).and_then(|mut file| {
        file.write_all(text.as_bytes())?;
        file.sync_all()
    });
    if let Err(error) = written.and_then(|()| fs::rename(&temporary, path)) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("{}: {error}", path.display()));
    }
    Ok(())
}

pub fn load_config() -> Config {
    config_path()
        .ok()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Config>(&bytes).ok())
        .map(sanitized)
        .unwrap_or_default()
}

fn sanitized(mut config: Config) -> Config {
    if !theme_known(&config.theme) {
        config.theme = default_theme();
    }
    if !effect_known(&config.effect) {
        config.effect = default_effect();
    }
    if !config.speed.is_finite() {
        config.speed = 1.0;
    }
    config.speed = config.speed.clamp(0.25, 3.0);
    config.brightness = config.brightness.clamp(5, 100);
    config
        .rules
        .retain(|rule| theme_known(&rule.theme) && effect_known(&rule.effect));
    config.rules.truncate(MAX_RULES);
    if config
        .games
        .as_ref()
        .is_some_and(|look| !theme_known(&look.theme) || !effect_known(&look.effect))
    {
        config.games = None;
    }
    config
}

fn save_config(config: &Config) -> Result<(), String> {
    let mut config = config.clone();
    config.schema = 1;
    let text = serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?;
    write_atomic(&config_path()?, &format!("{text}\n"))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct SavedDevice {
    mode: i32,
    colors: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct Handover {
    #[serde(default)]
    effects: Vec<String>,
    /// By [`Controller::key`].
    #[serde(default)]
    devices: BTreeMap<String, SavedDevice>,
}

impl Handover {
    fn is_empty(&self) -> bool {
        self.effects.is_empty() && self.devices.is_empty()
    }
}

fn handover_path() -> Result<PathBuf, String> {
    Ok(crate::equalizer::config_directory()?.join(HANDOVER_FILE))
}

fn load_handover() -> Handover {
    handover_path()
        .ok()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_handover(handover: &Handover) {
    if let Ok(path) = handover_path() {
        if handover.is_empty() {
            let _ = fs::remove_file(path);
        } else if let Ok(text) = serde_json::to_string(handover) {
            let _ = write_atomic(&path, &format!("{text}\n"));
        }
    }
}

/// OpenRGB's own SDK port (its "Server" setting), read only.
fn openrgb_port() -> Option<u16> {
    let home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let settings: Value =
        serde_json::from_slice(&fs::read(home.join("OpenRGB/OpenRGB.json")).ok()?).ok()?;
    let port = settings.pointer("/Server/default_port")?.as_u64()?;
    u16::try_from(port).ok().filter(|&port| port >= 1024)
}

fn port(config: &Config) -> u16 {
    std::env::var("SONICCORE_OPENRGB_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .or(config.port)
        .or_else(openrgb_port)
        .unwrap_or(openrgb_sdk::DEFAULT_PORT)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default()
}

// ---------------------------------------------------------------- colors

fn mix(a: Color, b: Color, amount: f32) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * amount).round() as u8;
    Color {
        r: channel(a.r, b.r),
        g: channel(a.g, b.g),
        b: channel(a.b, b.b),
    }
}

fn scale(color: Color, amount: f32) -> Color {
    mix(Color::BLACK, color, amount)
}

fn hsv(hue: f32, saturation: f32, value: f32) -> Color {
    let hue = hue.rem_euclid(1.0) * 6.0;
    let chroma = value * saturation;
    let x = chroma * (1.0 - ((hue % 2.0) - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let base = value - chroma;
    let channel = |value: f32| ((value + base) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color {
        r: channel(r),
        g: channel(g),
        b: channel(b),
    }
}

/// LEDs show pastel tones as near white: the saturation is pushed up and the
/// color brought to full brightness (white stays white).
fn vivid(color: Color) -> Color {
    let (r, g, b) = (
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max <= 0.0 {
        return color;
    }
    let saturation = (max - min) / max;
    if saturation < 0.12 {
        return Color { r: 255, g: 255, b: 255 };
    }
    let delta = max - min;
    let hue = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    } / 6.0;
    hsv(hue, 1.0 - (1.0 - saturation).powi(2), 1.0)
}

/// A point of the palette's gradient (`cyclic`: the last color leads back
/// to the first, for things that move).
fn sample(palette: &[Color], position: f32, cyclic: bool) -> Color {
    match palette.len() {
        0 => Color { r: 255, g: 255, b: 255 },
        1 => palette[0],
        count => {
            let spans = if cyclic { count } else { count - 1 };
            let at = if cyclic {
                position.rem_euclid(1.0)
            } else {
                position.clamp(0.0, 1.0)
            } * spans as f32;
            let index = (at.floor() as usize).min(spans - 1);
            mix(palette[index], palette[(index + 1) % count], at - index as f32)
        }
    }
}

fn parse_palette(colors: &[&str]) -> Vec<Color> {
    colors.iter().filter_map(|color| Color::parse(color)).collect()
}

// ---------------------------------------------------------------- layout

/// Where an LED sits on its device: across (x) and down (y), 0..1, and
/// which zone holds it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Led {
    x: f32,
    y: f32,
    /// Position along a strip (0 first LED .. 1 last), for meters.
    along: f32,
    /// A matrix zone's column, as a share of its width (meters use it).
    matrix: bool,
    single: bool,
}

fn layout(controller: &Controller) -> Vec<Led> {
    let total = controller.colors.len();
    let mut leds = vec![Led::default(); total];
    let mut offset = 0usize;
    for zone in &controller.zones {
        let count = zone.leds_count as usize;
        if offset + count > total {
            break;
        }
        match &zone.matrix {
            Some(matrix) if matrix.width > 0 && matrix.height > 0 => {
                for row in 0..matrix.height as usize {
                    for column in 0..matrix.width as usize {
                        let Some(&index) = matrix.map.get(row * matrix.width as usize + column) else {
                            continue;
                        };
                        let index = index as usize;
                        if index < count {
                            leds[offset + index] = Led {
                                x: (column as f32 + 0.5) / matrix.width as f32,
                                y: (row as f32 + 0.5) / matrix.height as f32,
                                along: 1.0 - (row as f32 + 0.5) / matrix.height as f32,
                                matrix: true,
                                single: false,
                            };
                        }
                    }
                }
            }
            _ => {
                for index in 0..count {
                    let along = if count > 1 {
                        index as f32 / (count - 1) as f32
                    } else {
                        0.5
                    };
                    leds[offset + index] = Led {
                        x: along,
                        y: 0.5,
                        along,
                        matrix: false,
                        single: zone.kind == 0 || count == 1,
                    };
                }
            }
        }
        offset += count;
    }
    // LEDs outside every zone (should not happen): spread them anyway.
    for (index, led) in leds.iter_mut().enumerate().skip(offset) {
        let along = index as f32 / total.max(1) as f32;
        *led = Led {
            x: along,
            y: 0.5,
            along,
            matrix: false,
            single: false,
        };
    }
    leds
}

// ---------------------------------------------------------------- effects

/// What the effects know of the sound: 32 band levels (low to high) and a
/// bass pulse, all 0..1 and already smoothed.
#[derive(Clone, Debug, Default)]
struct Sound {
    bands: Vec<f32>,
    bass: f32,
    level: f32,
}

impl Sound {
    fn band_at(&self, position: f32) -> f32 {
        if self.bands.is_empty() {
            return 0.0;
        }
        // The lower half of the spectrum holds most of the music.
        let usable = (self.bands.len() * 3 / 4).max(1);
        let index = ((position.clamp(0.0, 1.0) * (usable - 1) as f32).round() as usize).min(usable - 1);
        self.bands[index]
    }
}

struct Stars {
    /// Per LED: brightness and palette position of a twinkle.
    glow: Vec<(f32, f32)>,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// One device's frame. `time` already includes the speed; `place` is the
/// device's position among the others (0..1), so waves travel across them.
#[allow(clippy::too_many_arguments)]
fn render(
    effect: &str,
    palette: &[Color],
    leds: &[Led],
    time: f32,
    place: f32,
    sound: &Sound,
    stars: &mut Stars,
    rng: &mut Rng,
    step: f32,
) -> Vec<Color> {
    if stars.glow.len() != leds.len() {
        stars.glow = vec![(0.0, 0.0); leds.len()];
    }
    leds.iter()
        .enumerate()
        .map(|(index, led)| {
            // Still effects lay the palette along each device; a device of
            // one light takes its color from its place among the others.
            let spread = if led.single { place } else { led.x };
            match effect {
                "breathing" => {
                    let breath = 0.5 - 0.5 * (TAU * time / 4.0).cos();
                    scale(sample(palette, spread, led.single), 0.1 + 0.9 * breath)
                }
                "wave" => sample(palette, led.x * 0.8 + place * 0.25 - time * 0.22, true),
                "rainbow" => hsv(led.x * 0.8 + place * 0.25 - time * 0.18, 1.0, 1.0),
                "stars" => {
                    let star = &mut stars.glow[index];
                    star.0 *= (-step * 2.2).exp();
                    if rng.next() < step * 0.55 {
                        *star = (1.0, rng.next());
                    }
                    let base = scale(sample(palette, spread, true), 0.07);
                    mix(base, sample(palette, star.1, true), star.0)
                }
                "music" => {
                    let level = sound.band_at(led.x).max(sound.bass * 0.85);
                    let color = sample(palette, led.x * 0.7 + place * 0.3 + sound.bass * 0.15, true);
                    scale(color, 0.08 + 0.92 * level.clamp(0.0, 1.0))
                }
                "vu" => {
                    let lit = if led.matrix {
                        // A keyboard or panel: one bar per column.
                        led.along <= sound.band_at(led.x) + 0.02
                    } else if led.single {
                        true
                    } else {
                        led.along <= sound.level + 0.02
                    };
                    let color = sample(palette, if led.matrix { led.along } else { led.along * 0.999 }, false);
                    if led.single {
                        scale(color, 0.08 + 0.92 * sound.level)
                    } else if lit {
                        color
                    } else {
                        scale(color, 0.05)
                    }
                }
                _ => sample(palette, spread, led.single),
            }
        })
        .collect()
}

/// A short flash over every light: color, start and pulses.
struct Flash {
    color: Color,
    started: Instant,
    pulses: u32,
}

impl Flash {
    const PULSE: f32 = 0.42;

    /// How much of the flash shows now; None once it is over.
    fn amount(&self) -> Option<f32> {
        let elapsed = self.started.elapsed().as_secs_f32();
        let length = Self::PULSE * self.pulses as f32;
        (elapsed < length).then(|| (std::f32::consts::PI * (elapsed / Self::PULSE).fract()).sin())
    }
}

// ---------------------------------------------------------------- games

/// Whether a playing program is a game: started by Steam (its game id in
/// the environment), or a Windows program through Wine/Proton, or under
/// gamescope.
fn is_game(binary: &str, environment: &[u8]) -> bool {
    let binary = binary.to_ascii_lowercase();
    if binary.ends_with(".exe")
        || binary.contains("wine")
        || binary == "gamescope"
        || binary.starts_with("proton")
    {
        return true;
    }
    environment.split(|&byte| byte == 0).any(|entry| {
        let entry = String::from_utf8_lossy(entry);
        ["SteamGameId=", "SteamAppId="].iter().any(|key| {
            entry
                .strip_prefix(key)
                .is_some_and(|id| !id.is_empty() && id != "0")
        })
    })
}

fn stream_is_game(stream: &crate::pipewire::PlayingStream) -> bool {
    let environment = stream
        .pid
        .and_then(|pid| fs::read(format!("/proc/{pid}/environ")).ok())
        .unwrap_or_default();
    is_game(stream.binary.as_deref().unwrap_or_default(), &environment)
}

/// The look to show: the first rule whose application plays, then the
/// games rule when a game plays, then the chosen theme.
fn choose_look(config: &Config, playing: &[String], game: bool) -> (Look, Option<String>) {
    if let Some(rule) = config
        .rules
        .iter()
        .find(|rule| playing.iter().any(|app| app.eq_ignore_ascii_case(&rule.app)))
    {
        return (
            Look {
                theme: rule.theme.clone(),
                effect: rule.effect.clone(),
            },
            Some(rule.app.clone()),
        );
    }
    if game && let Some(look) = &config.games {
        return (look.clone(), Some("games".into()));
    }
    (
        Look {
            theme: config.theme.clone(),
            effect: config.effect.clone(),
        },
        None,
    )
}

// ---------------------------------------------------------------- headset

/// The headset's own effect for one of ours (it cannot be streamed to), and
/// its speed name.
fn headset_effect(effect: &str) -> &'static str {
    match effect {
        "breathing" => "breathing",
        "wave" | "rainbow" => "wave",
        "stars" => "glitch",
        _ => "solid",
    }
}

fn headset_speed(speed: f32) -> &'static str {
    if speed < 0.75 {
        "0.5"
    } else if speed < 1.25 {
        "1.0"
    } else if speed < 1.75 {
        "1.5"
    } else {
        "2.0"
    }
}

/// Ten colors for the headset (logo 1-5, then ring 1-5).
fn headset_colors(effect: &str, palette: &[Color], brightness: f32) -> Vec<String> {
    (0..10)
        .map(|index| {
            let position = (index % 5) as f32 / 5.0 + if index >= 5 { 0.1 } else { 0.0 };
            let color = if effect == "rainbow" {
                hsv(position, 1.0, 1.0)
            } else {
                vivid(sample(palette, position, true))
            };
            scale(color, brightness).hex()
        })
        .collect()
}

/// How long a change made to the headset from outside stays before the
/// theme goes back on it.
const HEADSET_REAPPLY: Duration = Duration::from_secs(4);

/// The headset's colors or effect are not the ones last sent.
fn headset_differs(state: &crate::state::RuntimeState, sent: &(Vec<String>, String, String)) -> bool {
    let actual: Vec<String> = [&state.logo_colors, &state.ring_colors]
        .into_iter()
        .flat_map(|zone| zone.iter().flatten())
        .map(|color| color.to_ascii_lowercase())
        .collect();
    actual.len() == sent.0.len() && actual != sent.0
        || state.logo_effect.as_deref().is_some_and(|effect| effect != sent.1)
}

fn run_set(feature: &str, value: &str) -> Result<bool, String> {
    crate::parse_set_command([feature.to_owned(), value.to_owned()].into_iter())
        .and_then(crate::set_control)
}

// ---------------------------------------------------------------- engine

struct Device {
    controller: Controller,
    leds: Vec<Led>,
    included: bool,
    custom: bool,
    sent: Vec<Color>,
    /// The last frames sent: the server may still show any of them.
    recent: VecDeque<Vec<Color>>,
    stars: Stars,
}

#[derive(Default, Serialize)]
struct DeviceStatus {
    key: String,
    name: String,
    kind: &'static str,
    leds: usize,
    included: bool,
}

/// An automatic gain: where a level sits within its own recent range
/// (about the last few seconds), so the lights move with the music at any
/// volume instead of staying full when it is loud.
#[derive(Clone, Copy, Debug, Default)]
struct AutoRange {
    low: f32,
    high: f32,
    /// Has a value: the first one sets the range (and a silence clears it).
    primed: bool,
}

impl AutoRange {
    /// Below this span the sound is steady (or silent): nothing to show.
    const MIN_SPAN: f32 = 0.12;
    /// How fast the range follows a quieter or louder passage, per frame.
    const FOLLOW: f32 = 0.012;

    fn place(&mut self, value: f32) -> f32 {
        if !self.primed {
            *self = Self {
                low: value,
                high: value,
                primed: true,
            };
        }
        self.high = if value > self.high {
            value
        } else {
            self.high + (value - self.high) * Self::FOLLOW
        };
        self.low = if value < self.low {
            value
        } else {
            self.low + (value - self.low) * Self::FOLLOW
        };
        let span = (self.high - self.low).max(Self::MIN_SPAN);
        ((value - self.low) / span).clamp(0.0, 1.0)
    }
}

struct Meter {
    connection: Option<zbus::blocking::Connection>,
    retry_at: Instant,
    sound: Sound,
    band_ranges: Vec<AutoRange>,
    bass_range: AutoRange,
    level_range: AutoRange,
}

impl Meter {
    fn new() -> Self {
        Self {
            connection: None,
            retry_at: Instant::now(),
            sound: Sound::default(),
            band_ranges: Vec::new(),
            bass_range: AutoRange::default(),
            level_range: AutoRange::default(),
        }
    }

    /// One frame from the spectrum meter (started on demand by D-Bus).
    fn update(&mut self) {
        if self.connection.is_none() && Instant::now() >= self.retry_at {
            self.connection = zbus::blocking::Connection::session().ok();
            self.retry_at = Instant::now() + RETRY;
        }
        let frame = self.connection.as_ref().and_then(|connection| {
            connection
                .call_method(
                    Some("org.jambalinux.soniccore.Meter"),
                    "/org/jambalinux/soniccore/Meter",
                    Some("org.jambalinux.soniccore.Meter"),
                    "Frame",
                    &(),
                )
                .ok()
                .and_then(|reply| reply.body().deserialize::<(Vec<f64>, f64, bool)>().ok())
        });
        let (bands, active) = match frame {
            Some((bands, _, active)) => (
                bands.into_iter().map(|level| level as f32).collect::<Vec<_>>(),
                active,
            ),
            None => (Vec::new(), false),
        };
        self.listen(&bands, active);
    }

    /// The levels the effects use: each placed in its own recent range,
    /// then smoothed (fast attack, slow release).
    fn listen(&mut self, bands: &[f32], active: bool) {
        let smooth = |old: f32, new: f32| {
            let rate = if new > old { 0.6 } else { 0.14 };
            old + (new - old) * rate
        };
        let count = bands.len().max(32);
        if !active {
            // The next sound starts its own ranges.
            self.band_ranges.iter_mut().for_each(|range| range.primed = false);
            self.bass_range.primed = false;
            self.level_range.primed = false;
        }
        if self.sound.bands.len() != count {
            self.sound.bands = vec![0.0; count];
            self.band_ranges = vec![AutoRange::default(); count];
        }
        for index in 0..count {
            let target = if active {
                self.band_ranges[index].place(bands.get(index).copied().unwrap_or_default())
            } else {
                0.0
            };
            self.sound.bands[index] = smooth(self.sound.bands[index], target);
        }
        let average = |slice: &[f32]| {
            if slice.is_empty() {
                0.0
            } else {
                slice.iter().sum::<f32>() / slice.len() as f32
            }
        };
        let (bass, level) = if active {
            (
                self.bass_range.place(average(&bands[..bands.len().min(6)])),
                self.level_range
                    .place(average(&bands[..(bands.len() * 3 / 4).max(bands.len().min(1))])),
            )
        } else {
            (0.0, 0.0)
        };
        self.sound.bass = smooth(self.sound.bass, bass);
        self.sound.level = smooth(self.sound.level, level);
    }
}

struct Engine {
    config: Config,
    config_stamp: Option<SystemTime>,
    live: Value,
    live_stamp: Option<SystemTime>,
    client: Option<Client>,
    devices: Vec<Device>,
    profiles: Vec<String>,
    error: Option<String>,
    problem: Option<&'static str>,
    retry_at: Instant,
    released: bool,
    time: f32,
    last_frame: Instant,
    rng: Rng,
    meter: Meter,
    playing: Vec<String>,
    game: bool,
    apps_checked: Option<Instant>,
    look: Look,
    rule: Option<String>,
    headset: crate::state::RuntimeState,
    headset_checked: Option<Instant>,
    headset_sent: Option<(Vec<String>, String, String)>,
    /// When the theme was last written to the headset.
    headset_written: Option<Instant>,
    headset_retry: Instant,
    microphone: Option<String>,
    battery_warned: Option<Instant>,
    flash: Option<Flash>,
    conflict: bool,
    conflict_checked: Instant,
    mismatches: u32,
    /// What to put back when OpenRGB takes the lights again.
    handover: Handover,
    /// The Effects plugin's index in OpenRGB, when it is loaded.
    effects_plugin: Option<u32>,
    /// This connection's effects were paused.
    taken: bool,
    /// Effects started in OpenRGB while Sonic Core is in charge.
    foreign_effects: Vec<String>,
    effects_checked: Option<Instant>,
    /// When effects were last started or stopped, or a profile loaded.
    effects_changed: Option<Instant>,
    connected_at: Instant,
    /// A profile to hand the lights over with, and the last request done
    /// (its id), for the command that asked.
    request: Option<(String, u64)>,
    profile_loaded: Option<u64>,
    /// Seconds since the last frame (already drawn effects use it).
    step: f32,
    preview_stars: Stars,
    preview_sent: Option<(Instant, Vec<Color>)>,
    status_written: Option<Instant>,
}

impl Engine {
    fn new() -> Self {
        let config = load_config();
        let look = Look {
            theme: config.theme.clone(),
            effect: config.effect.clone(),
        };
        Self {
            config,
            config_stamp: None,
            live: Value::Null,
            live_stamp: None,
            client: None,
            devices: Vec::new(),
            profiles: Vec::new(),
            error: None,
            problem: None,
            retry_at: Instant::now(),
            released: true,
            time: 0.0,
            last_frame: Instant::now(),
            rng: Rng(0x2545_f491_4f6c_dd1d ^ u64::from(std::process::id())),
            meter: Meter::new(),
            playing: Vec::new(),
            game: false,
            apps_checked: None,
            look,
            rule: None,
            headset: crate::state::RuntimeState::default(),
            headset_checked: None,
            headset_sent: None,
            headset_written: None,
            headset_retry: Instant::now(),
            microphone: None,
            battery_warned: None,
            flash: None,
            conflict: false,
            conflict_checked: Instant::now(),
            mismatches: 0,
            handover: load_handover(),
            effects_plugin: None,
            taken: false,
            foreign_effects: Vec::new(),
            effects_checked: None,
            effects_changed: None,
            connected_at: Instant::now(),
            request: None,
            profile_loaded: None,
            step: 0.0,
            preview_stars: Stars { glow: Vec::new() },
            preview_sent: None,
            status_written: None,
        }
    }

    fn reload(&mut self) {
        if let Ok(path) = runtime_path(REQUEST_FILE)
            && let Ok(bytes) = fs::read(&path)
        {
            let _ = fs::remove_file(&path);
            self.request = serde_json::from_slice::<Value>(&bytes).ok().and_then(|value| {
                let name = value["profile"].as_str().filter(|name| !name.is_empty())?;
                Some((name.to_owned(), value["id"].as_u64().unwrap_or_default()))
            });
        }
        let stamp = config_path()
            .ok()
            .and_then(|path| fs::metadata(path).ok())
            .and_then(|metadata| metadata.modified().ok());
        if stamp != self.config_stamp || self.config_stamp.is_none() {
            self.config_stamp = stamp;
            let config = load_config();
            if config.excluded != self.config.excluded {
                for device in &mut self.devices {
                    device.included = !config.excluded.contains(&device.controller.key());
                    device.sent.clear();
                }
            }
            self.config = config;
        }
        let stamp = runtime_path(LIVE_FILE)
            .ok()
            .and_then(|path| fs::metadata(path).ok())
            .and_then(|metadata| metadata.modified().ok());
        if stamp != self.live_stamp {
            self.live_stamp = stamp;
            self.live = runtime_path(LIVE_FILE)
                .ok()
                .and_then(|path| fs::read(path).ok())
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or(Value::Null);
        }
    }

    fn palette(&self, theme: &str) -> Vec<Color> {
        let colors = match theme {
            "headset" => {
                let mut colors = Vec::new();
                for zone in [&self.headset.logo_colors, &self.headset.ring_colors] {
                    colors.extend(zone.iter().flatten().filter_map(|color| Color::parse(color)));
                }
                colors.dedup();
                colors
            }
            "wallpaper" | "cover" => self.live[theme]
                .as_array()
                .map(|colors| {
                    colors
                        .iter()
                        .filter_map(Value::as_str)
                        .filter_map(Color::parse)
                        .collect()
                })
                .unwrap_or_default(),
            id => THEMES
                .iter()
                .find(|theme| theme.id == id)
                .map(|theme| parse_palette(theme.colors))
                .unwrap_or_default(),
        };
        if colors.is_empty() {
            parse_palette(THEMES[0].colors)
        } else {
            colors.into_iter().map(vivid).collect()
        }
    }

    fn connect(&mut self) {
        if Instant::now() < self.retry_at {
            return;
        }
        self.retry_at = Instant::now() + RETRY;
        let attempt = Client::connect(port(&self.config), CLIENT_NAME).and_then(|mut client| {
            let controllers = client.controllers()?;
            let profiles = client.profiles().unwrap_or_default();
            Ok((client, controllers, profiles))
        });
        match attempt {
            Ok((client, controllers, profiles)) => {
                self.client = Some(client);
                self.profiles = profiles;
                self.taken = false;
                self.effects_plugin = None;
                self.connected_at = Instant::now();
                self.adopt(controllers);
                self.error = None;
                self.problem = None;
            }
            Err(error) => {
                self.problem = Some(if error.contains("not running") {
                    "server-off"
                } else {
                    "server-error"
                });
                self.error = Some(error);
                self.client = None;
                self.devices.clear();
            }
        }
    }

    fn adopt(&mut self, controllers: Vec<Controller>) {
        self.devices = controllers
            .into_iter()
            .filter(|controller| !controller.location.starts_with(OWN_DEVICE_LOCATION))
            .filter(|controller| !controller.colors.is_empty())
            .map(|controller| Device {
                leds: layout(&controller),
                included: !self.config.excluded.contains(&controller.key()),
                custom: false,
                sent: Vec::new(),
                recent: VecDeque::new(),
                stars: Stars { glow: Vec::new() },
                controller,
            })
            .collect();
    }

    fn lost(&mut self, error: String) {
        self.client = None;
        self.devices.clear();
        self.problem = Some("server-error");
        self.error = Some(error);
        self.retry_at = Instant::now() + RETRY;
    }

    /// Back to OpenRGB: what it showed before (each device's mode and
    /// colors, its running effects), or the profile chosen for it.
    fn release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        for device in &mut self.devices {
            device.custom = false;
            device.sent.clear();
            device.recent.clear();
        }
        self.taken = false;
        self.foreign_effects.clear();
        match self.config.restore_profile.clone() {
            Some(profile) if self.profiles.contains(&profile) => {
                // The profile sets every device (and its own effects).
                self.handover = Handover::default();
                save_handover(&self.handover);
                if let Some(client) = self.client.as_mut()
                    && let Err(error) = client.load_profile(&profile)
                {
                    self.lost(error);
                }
            }
            _ => self.restore_openrgb(),
        }
    }

    /// Puts back what OpenRGB showed before Sonic Core took the lights.
    fn restore_openrgb(&mut self) {
        if self.handover.is_empty() || self.client.is_none() || !self.settled() {
            return;
        }
        let Some(client) = self.client.as_mut() else {
            return;
        };
        let controllers = match client.controllers() {
            Ok(controllers) => controllers,
            Err(error) => return self.lost(error),
        };
        for controller in &controllers {
            let Some(saved) = self.handover.devices.get(&controller.key()) else {
                continue;
            };
            let colors: Vec<Color> = saved.colors.iter().filter_map(|color| Color::parse(color)).collect();
            let per_led = usize::try_from(saved.mode)
                .ok()
                .and_then(|index| controller.modes.get(index))
                .is_some_and(|mode| mode.color_mode == 1);
            let result = client.update_mode(controller, saved.mode).and_then(|()| {
                if per_led && colors.len() == controller.colors.len() {
                    client.update_leds(controller.index, &colors)
                } else {
                    Ok(())
                }
            });
            if let Err(error) = result {
                return self.lost(error);
            }
        }
        if !self.handover.effects.is_empty()
            && let Some(plugin) = self.find_effects_plugin()
            && let Some(client) = self.client.as_mut()
        {
            for name in &self.handover.effects.clone() {
                self.effects_changed = Some(Instant::now());
                if let Err(error) = client.set_effect(plugin, name, true) {
                    return self.lost(error);
                }
            }
        }
        // Without the plugin (removed meanwhile) there is nothing to start.
        self.handover = Handover::default();
        save_handover(&self.handover);
    }

    /// Whether OpenRGB had time to settle since the last change to its
    /// effects (and since connecting).
    fn settled(&self) -> bool {
        self.connected_at.elapsed() >= CONNECT_SETTLE
            && self
                .effects_changed
                .is_none_or(|changed| changed.elapsed() >= EFFECTS_SETTLE)
    }

    /// The user picked one of OpenRGB's profiles: OpenRGB gets the lights
    /// with it. The paused effects are not started first (the profile
    /// brings its own, and loading it over an effect that just started is
    /// what crashed OpenRGB).
    fn hand_over_with_profile(&mut self, profile: &str, id: u64) {
        self.released = true;
        for device in &mut self.devices {
            device.custom = false;
            device.sent.clear();
            device.recent.clear();
        }
        self.taken = false;
        self.foreign_effects.clear();
        self.handover = Handover::default();
        save_handover(&self.handover);
        self.effects_changed = Some(Instant::now());
        if let Some(client) = self.client.as_mut() {
            match client.load_profile(profile) {
                Ok(()) => self.profile_loaded = Some(id),
                Err(error) => self.lost(error),
            }
        }
    }

    fn find_effects_plugin(&mut self) -> Option<u32> {
        if self.effects_plugin.is_none() {
            let client = self.client.as_mut()?;
            self.effects_plugin = client
                .plugins()
                .ok()?
                .into_iter()
                .find(|plugin| plugin.name == openrgb_sdk::EFFECTS_PLUGIN && plugin.sdk_version >= 1)
                .map(|plugin| plugin.index);
        }
        self.effects_plugin
    }

    /// Taking the lights: the OpenRGB Effects plugin's running effects
    /// pause (they would write over ours) and are remembered, and every
    /// device is read again so its current mode and colors can be saved
    /// before the first frame.
    fn take_over(&mut self) {
        if self.taken || self.client.is_none() || !self.settled() {
            return;
        }
        self.taken = true;
        if let Some(plugin) = self.find_effects_plugin()
            && let Some(client) = self.client.as_mut()
            && let Ok(effects) = client.effects(plugin)
        {
            for effect in effects.iter().filter(|effect| effect.running) {
                self.effects_changed = Some(Instant::now());
                if let Err(error) = client.set_effect(plugin, &effect.name, false) {
                    return self.lost(error);
                }
                if !self.handover.effects.contains(&effect.name) {
                    self.handover.effects.push(effect.name.clone());
                }
            }
            save_handover(&self.handover);
        }
        if let Some(client) = self.client.as_mut() {
            match client.controllers() {
                Ok(controllers) => self.adopt(controllers),
                Err(error) => self.lost(error),
            }
        }
    }

    /// An effect started in OpenRGB while Sonic Core is in charge: the
    /// user's choice, so it is reported rather than stopped.
    fn watch_effects(&mut self) {
        if self
            .effects_checked
            .is_some_and(|checked| checked.elapsed() < Duration::from_secs(5))
        {
            return;
        }
        self.effects_checked = Some(Instant::now());
        let Some(plugin) = self.find_effects_plugin() else {
            self.foreign_effects.clear();
            return;
        };
        if let Some(client) = self.client.as_mut()
            && let Ok(effects) = client.effects(plugin)
        {
            self.foreign_effects = effects
                .into_iter()
                .filter(|effect| effect.running)
                .map(|effect| effect.name)
                .collect();
        }
    }

    fn watch_headset(&mut self) {
        if self
            .headset_checked
            .is_some_and(|checked| checked.elapsed() < Duration::from_millis(500))
        {
            return;
        }
        self.headset_checked = Some(Instant::now());
        let Ok(state) = crate::state::load() else {
            return;
        };
        if self.config.alerts && self.config.active {
            if self.microphone.is_some() && state.microphone != self.microphone {
                self.flash = Some(match state.microphone.as_deref() {
                    Some("muted") => Flash {
                        color: Color { r: 255, g: 0, b: 0 },
                        started: Instant::now(),
                        pulses: 2,
                    },
                    _ => Flash {
                        color: Color { r: 0, g: 255, b: 60 },
                        started: Instant::now(),
                        pulses: 1,
                    },
                });
            }
            let low = state.headset_connected == Some(true)
                && state.charging != Some(true)
                && state.battery_percent.is_some_and(|percent| percent <= 15);
            if low
                && self
                    .battery_warned
                    .is_none_or(|warned| warned.elapsed() > Duration::from_secs(300))
            {
                self.battery_warned = Some(Instant::now());
                self.flash = Some(Flash {
                    color: Color { r: 255, g: 150, b: 0 },
                    started: Instant::now(),
                    pulses: 3,
                });
            }
        }
        if state.headset_connected != self.headset.headset_connected {
            // Switched on: the theme goes to it again.
            self.headset_sent = None;
            self.headset_retry = Instant::now();
        }
        self.microphone = state.microphone.clone();
        self.headset = state;
    }

    fn watch_apps(&mut self) {
        let needed = !self.config.rules.is_empty() || self.config.games.is_some();
        if !needed {
            self.playing.clear();
            self.game = false;
        }
        if self
            .apps_checked
            .is_some_and(|checked| checked.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        self.apps_checked = Some(Instant::now());
        if let Ok(streams) = crate::pipewire::playing_streams() {
            let mut names: Vec<String> = Vec::new();
            for stream in &streams {
                if !names.contains(&stream.app) {
                    names.push(stream.app.clone());
                }
            }
            names.sort_by_key(|name| name.to_lowercase());
            self.game = self.config.games.is_some() && streams.iter().any(stream_is_game);
            self.playing = names;
        }
    }

    fn update_headset(&mut self) {
        // Its light switched off by the user stays off; switched on again,
        // it still shows the theme.
        if !self.config.headset
            || self.look.theme == "headset"
            || self.headset.headset_connected != Some(true)
            || self.headset.lighting_enabled == Some(false)
            || Instant::now() < self.headset_retry
        {
            return;
        }
        // Following the theme, the headset keeps it: changed from outside
        // (the lighting card, OpenRGB), the theme goes back on it after a
        // moment.
        if let Some(sent) = &self.headset_sent
            && self
                .headset_written
                .is_some_and(|written| written.elapsed() >= HEADSET_REAPPLY)
            && headset_differs(&self.headset, sent)
        {
            self.headset_sent = None;
        }
        let palette = self.palette(&self.look.theme);
        let brightness = f32::from(self.config.brightness) / 100.0;
        let wanted = (
            headset_colors(&self.look.effect, &palette, brightness),
            headset_effect(&self.look.effect).to_owned(),
            headset_speed(self.config.speed).to_owned(),
        );
        if self.headset_sent.as_ref() == Some(&wanted) {
            return;
        }
        let previous = self.headset_sent.clone();
        let mut result = Ok(true);
        if previous.as_ref().is_none_or(|sent| sent.0 != wanted.0) {
            result = run_set("leds", &wanted.0.join(","));
        }
        if result == Ok(true) && previous.as_ref().is_none_or(|sent| sent.1 != wanted.1) {
            result = run_set("effect", &wanted.1);
        }
        if result == Ok(true)
            && wanted.1 != "solid"
            && previous.as_ref().is_none_or(|sent| sent.2 != wanted.2 || sent.1 != wanted.1)
        {
            result = run_set("speed", &wanted.2);
        }
        if result == Ok(true) {
            self.headset_sent = Some(wanted);
            self.headset_written = Some(Instant::now());
        } else {
            // Off, asleep, held or saving battery: try again later.
            self.headset_sent = None;
            self.headset_retry = Instant::now() + Duration::from_secs(15);
        }
    }

    /// The effects' clock and the sound, once per frame.
    fn advance(&mut self) {
        self.step = self.last_frame.elapsed().as_secs_f32().min(0.25);
        self.last_frame = Instant::now();
        self.time += self.step * self.config.speed;
        if effect_needs_sound(&self.look.effect) {
            self.meter.update();
        }
    }

    /// The effect on a strip of ten lights at full brightness: what the
    /// app's own effects follow while Sonic Core is in charge.
    fn publish_preview(&mut self) {
        let leds: Vec<Led> = (0..10)
            .map(|index| {
                let along = (index % 5) as f32 / 4.0;
                Led {
                    x: along * 0.9 + if index >= 5 { 0.1 } else { 0.0 },
                    y: 0.5,
                    along,
                    matrix: false,
                    single: false,
                }
            })
            .collect();
        let palette = self.palette(&self.look.theme);
        let mut frame = render(
            &self.look.effect,
            &palette,
            &leds,
            self.time,
            0.0,
            &self.meter.sound,
            &mut self.preview_stars,
            &mut self.rng,
            self.step * self.config.speed,
        );
        if let Some((color, amount)) = self.flash.as_ref().and_then(|flash| flash.amount().map(|amount| (flash.color, amount))) {
            for light in &mut frame {
                *light = mix(*light, color, amount);
            }
        }
        if self
            .preview_sent
            .as_ref()
            .is_some_and(|(sent, colors)| sent.elapsed() < FRAME_PUBLISH || *colors == frame)
        {
            return;
        }
        let text = json!({
            "schema": 1,
            "off": false,
            "colors": frame.iter().map(|color| color.hex()).collect::<Vec<_>>(),
        });
        if let Ok(path) = runtime_path(FRAME_FILE) {
            let _ = write_atomic(&path, &format!("{text}\n"));
        }
        self.preview_sent = Some((Instant::now(), frame));
    }

    fn withdraw_preview(&mut self) {
        if self.preview_sent.take().is_some()
            && let Ok(path) = runtime_path(FRAME_FILE)
        {
            let _ = fs::remove_file(path);
        }
    }

    fn draw(&mut self) {
        let step = self.step;
        let palette = self.palette(&self.look.theme);
        let brightness = f32::from(self.config.brightness) / 100.0;
        let flash = self.flash.as_ref().and_then(|flash| flash.amount().map(|amount| (flash.color, amount)));
        if flash.is_none() {
            self.flash = None;
        }
        let included = self.devices.iter().filter(|device| device.included).count().max(1);
        let mut place = 0usize;
        let mut failure = None;
        let mut snapshots = false;
        for device in &mut self.devices {
            if !device.included {
                continue;
            }
            let Some(client) = self.client.as_mut() else {
                return;
            };
            let mut frame = render(
                &self.look.effect,
                &palette,
                &device.leds,
                self.time,
                place as f32 / included as f32,
                &self.meter.sound,
                &mut device.stars,
                &mut self.rng,
                step * self.config.speed,
            );
            place += 1;
            for color in &mut frame {
                *color = scale(*color, brightness);
                if let Some((flash_color, amount)) = flash {
                    *color = mix(*color, flash_color, amount);
                }
            }
            if !device.custom {
                // What it shows now, to put back when OpenRGB takes over.
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    self.handover.devices.entry(device.controller.key())
                {
                    entry.insert(SavedDevice {
                        mode: device.controller.active_mode,
                        colors: device.controller.colors.iter().map(|color| color.hex()).collect(),
                    });
                    snapshots = true;
                }
                if let Err(error) = client.set_custom_mode(device.controller.index) {
                    failure = Some(error);
                    break;
                }
                device.custom = true;
                device.sent.clear();
            }
            if frame != device.sent {
                if let Err(error) = client.update_leds(device.controller.index, &frame) {
                    failure = Some(error);
                    break;
                }
                if device.recent.len() >= RECENT_FRAMES {
                    device.recent.pop_front();
                }
                device.recent.push_back(frame.clone());
                device.sent = frame;
            }
        }
        if snapshots {
            save_handover(&self.handover);
        }
        if let Some(error) = failure {
            self.lost(error);
        }
    }

    /// Another program (an OpenRGB effect) writing the same lights makes
    /// them flicker: noticed when a device shows none of our last frames,
    /// twice in a row, or when an OpenRGB effect runs.
    fn check_conflict(&mut self) {
        if self.conflict_checked.elapsed() < Duration::from_secs(4) {
            return;
        }
        self.conflict_checked = Instant::now();
        let Some(client) = self.client.as_mut() else {
            self.mismatches = 0;
            self.conflict = false;
            return;
        };
        let Ok(controllers) = client.controllers() else {
            return;
        };
        let mismatch = self.devices.iter().any(|device| {
            device.included
                && !device.recent.is_empty()
                && controllers
                    .iter()
                    .find(|controller| controller.index == device.controller.index)
                    .is_some_and(|controller| !device.recent.contains(&controller.colors))
        });
        self.mismatches = if mismatch { self.mismatches + 1 } else { 0 };
        self.conflict = self.mismatches >= 2 || !self.foreign_effects.is_empty();
    }

    fn write_status(&mut self, force: bool) {
        if !force
            && self
                .status_written
                .is_some_and(|written| written.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.status_written = Some(Instant::now());
        let devices: Vec<DeviceStatus> = self
            .devices
            .iter()
            .map(|device| DeviceStatus {
                key: device.controller.key(),
                name: device.controller.name.clone(),
                kind: openrgb_sdk::device_kind(device.controller.kind),
                leds: device.controller.colors.len(),
                included: device.included,
            })
            .collect();
        let status = json!({
            "schema": 1,
            "updated_at_ms": now_ms() as u64,
            "connected": self.client.is_some(),
            "protocol": self.client.as_ref().map(Client::protocol),
            "port": port(&self.config),
            "problem": self.problem,
            "error": self.error,
            "devices": devices,
            "profiles": self.profiles,
            "theme": self.look.theme,
            "effect": self.look.effect,
            "rule": self.rule,
            "playing": self.playing,
            "game": self.game,
            "conflict": self.conflict,
            "in_charge": !self.released,
            "profile_loaded": self.profile_loaded,
            "openrgb_paused": self.handover.effects,
            "openrgb_effects": self.foreign_effects,
            "headset_applied": self.headset_sent.is_some(),
        });
        // Rewritten once a second even when nothing changed, so readers can
        // tell the engine is alive.
        let text = format!("{status}\n");
        if let Ok(path) = runtime_path(STATUS_FILE) {
            let _ = write_atomic(&path, &text);
        }
    }

    fn tick(&mut self) -> Duration {
        self.reload();
        self.watch_headset();
        self.watch_apps();
        let (look, rule) = choose_look(&self.config, &self.playing, self.game);
        if look != self.look {
            self.look = look;
        }
        self.rule = rule;
        if let Some(client) = self.client.as_mut() {
            match client.poll() {
                Ok(()) if client.devices_changed => match client.controllers() {
                    Ok(controllers) => self.adopt(controllers),
                    Err(error) => self.lost(error),
                },
                Ok(()) => {}
                Err(error) => self.lost(error),
            }
        }
        if self.client.is_none() {
            self.connect();
        }
        // A picked profile is handled once Sonic Core is no longer in charge
        // (the app writes the request before switching), instead of the
        // usual hand-back.
        if !self.config.active
            && self.client.is_some()
            && self.settled()
            && let Some((profile, id)) = self.request.take()
        {
            self.hand_over_with_profile(&profile, id);
            self.write_status(true);
        }
        if !self.config.active {
            self.withdraw_preview();
            if self.request.is_none() {
                self.release();
            }
            // Handed back while the server was away: done once it is back.
            self.restore_openrgb();
            self.conflict = false;
            self.write_status(false);
            return IDLE;
        }
        self.released = false;
        self.take_over();
        self.watch_effects();
        self.update_headset();
        self.advance();
        self.publish_preview();
        // Not before OpenRGB's effects are paused: both would write.
        if self.taken {
            self.draw();
        }
        self.check_conflict();
        self.write_status(false);
        if self.client.is_some() { FRAME } else { IDLE }
    }
}

/// `soniccore lights run`: the engine, until stopped.
pub fn run() -> Result<bool, String> {
    let mut engine = Engine::new();
    loop {
        let started = Instant::now();
        let wait = engine.tick();
        thread::sleep(wait.saturating_sub(started.elapsed()));
    }
}

// ---------------------------------------------------------------- commands

fn catalog() -> Value {
    json!({
        "themes": THEMES.iter().map(|theme| json!({
            "id": theme.id,
            "name": theme.name,
            "colors": theme.colors,
        })).chain(LIVE_THEMES.iter().map(|(id, name)| json!({
            "id": id,
            "name": name,
            "live": true,
        }))).collect::<Vec<_>>(),
        "effects": EFFECTS.iter().map(|(id, name)| json!({
            "id": id,
            "name": name,
            "sound": effect_needs_sound(id),
        })).collect::<Vec<_>>(),
    })
}

fn report() -> Value {
    let config = load_config();
    let status: Value = runtime_path(STATUS_FILE)
        .ok()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null);
    let running = status["updated_at_ms"]
        .as_u64()
        .is_some_and(|updated| now_ms().saturating_sub(u128::from(updated)) < STALE_MS);
    let mut report = json!({
        "schema": 1,
        "running": running,
        "config": config,
        "status": if running { status } else { Value::Null },
    });
    if let (Some(report), Value::Object(catalog)) = (report.as_object_mut(), catalog()) {
        report.extend(catalog);
    }
    report
}

fn print_report(json: bool) {
    let report = report();
    if json {
        println!("{report}");
        return;
    }
    let config = &report["config"];
    println!(
        "Luzes do PC: {}",
        if config["active"].as_bool() == Some(true) {
            "Sonic Core no comando"
        } else {
            "OpenRGB no comando"
        }
    );
    println!("tema: {}  efeito: {}", config["theme"], config["effect"]);
    let status = &report["status"];
    if report["running"].as_bool() != Some(true) {
        println!("serviço: parado");
    } else if status["connected"].as_bool() == Some(true) {
        for device in status["devices"].as_array().into_iter().flatten() {
            println!(
                "  {} ({} LEDs){}",
                device["name"].as_str().unwrap_or_default(),
                device["leds"],
                if device["included"].as_bool() == Some(true) { "" } else { " — fica com o OpenRGB" }
            );
        }
    } else {
        println!("OpenRGB: {}", status["error"].as_str().unwrap_or("sem conexão"));
    }
}

fn on_off(value: Option<&str>) -> Result<bool, String> {
    match value {
        Some("on") => Ok(true),
        Some("off") => Ok(false),
        _ => Err("expected on or off".into()),
    }
}

fn validated_app(app: &str) -> Result<String, String> {
    let app = app.trim();
    if app.is_empty() || app.len() > 128 || app.chars().any(char::is_control) {
        return Err("invalid application name".into());
    }
    Ok(app.to_owned())
}

fn look_from(theme: &str, effect: &str) -> Result<Look, String> {
    if !theme_known(theme) {
        return Err(format!("unknown theme: {theme}"));
    }
    if !effect_known(effect) {
        return Err(format!("unknown effect: {effect}"));
    }
    Ok(Look {
        theme: theme.into(),
        effect: effect.into(),
    })
}

/// Applies one `soniccore lights` change to a configuration.
fn change(mut config: Config, words: &[String]) -> Result<Config, String> {
    let word = |index: usize| words.get(index).map(String::as_str);
    match (word(0), words.len()) {
        (Some("on"), 1) => config.active = true,
        (Some("off"), 1) => config.active = false,
        (Some("theme"), 2) => config.theme = look_from(&words[1], "solid")?.theme,
        (Some("effect"), 2) => config.effect = look_from("synthwave", &words[1])?.effect,
        (Some("speed"), 2) => {
            let speed: f32 = words[1].parse().map_err(|_| "speed must be a number")?;
            if !(0.25..=3.0).contains(&speed) {
                return Err("speed must be between 0.25 and 3".into());
            }
            config.speed = speed;
        }
        (Some("brightness"), 2) => {
            let brightness: u8 = words[1].parse().map_err(|_| "brightness must be 5 to 100")?;
            if !(5..=100).contains(&brightness) {
                return Err("brightness must be 5 to 100".into());
            }
            config.brightness = brightness;
        }
        (Some("headset"), 2) => config.headset = on_off(word(1))?,
        (Some("alerts"), 2) => config.alerts = on_off(word(1))?,
        (Some("device"), 3) => {
            let key = words[1].trim();
            if key.is_empty() || key.len() > 512 {
                return Err("invalid device".into());
            }
            if on_off(word(2))? {
                config.excluded.remove(key);
            } else {
                config.excluded.insert(key.to_owned());
            }
        }
        (Some("rule"), 3) if word(2) == Some("remove") => {
            let app = validated_app(&words[1])?;
            config.rules.retain(|rule| !rule.app.eq_ignore_ascii_case(&app));
        }
        (Some("rule"), 4) => {
            let app = validated_app(&words[1])?;
            let look = look_from(&words[2], &words[3])?;
            config.rules.retain(|rule| !rule.app.eq_ignore_ascii_case(&app));
            if config.rules.len() >= MAX_RULES {
                return Err("too many rules".into());
            }
            config.rules.push(Rule {
                app,
                theme: look.theme,
                effect: look.effect,
            });
        }
        (Some("games"), 2) if word(1) == Some("off") => config.games = None,
        (Some("games"), 3) => config.games = Some(look_from(&words[1], &words[2])?),
        (Some("restore"), 2) => {
            config.restore_profile = match words[1].as_str() {
                "" | "openrgb" => None,
                name if name.len() <= 256 && !name.chars().any(char::is_control) => Some(name.into()),
                _ => return Err("invalid profile name".into()),
            }
        }
        _ => return Err(usage().into()),
    }
    Ok(config)
}

fn usage() -> &'static str {
    "usage: soniccore lights [--format json] | lights run | lights on|off | lights theme <id> | \
     lights effect <id> | lights speed <0.25-3> | lights brightness <5-100> | \
     lights headset on|off | lights alerts on|off | lights device <key> on|off | \
     lights rule <app> <theme> <effect> | lights rule <app> remove | \
     lights games <theme> <effect> | lights games off | lights restore <profile|openrgb> | \
     lights live <wallpaper|cover> <#rrggbb...> | lights profile <name>"
}

/// `soniccore lights live wallpaper|cover #rrggbb ...`: the app's colors
/// for the live themes (in the runtime directory: gone at logout).
fn set_live(kind: &str, colors: &[String]) -> Result<(), String> {
    if !matches!(kind, "wallpaper" | "cover") {
        return Err("live colors are for wallpaper or cover".into());
    }
    let parsed: Vec<String> = colors
        .iter()
        .take(MAX_LIVE_COLORS)
        .map(|color| Color::parse(color).map(Color::hex).ok_or(format!("invalid color: {color}")))
        .collect::<Result<_, _>>()?;
    let path = runtime_path(LIVE_FILE)?;
    let mut live: Value = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    live[kind] = json!(parsed);
    write_atomic(&path, &format!("{live}\n"))
}

/// `soniccore lights profile <name>`: OpenRGB takes the lights back and
/// loads one of its profiles.
fn load_profile(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err("invalid profile name".into());
    }
    let mut config = load_config();
    let report_now = report();
    if report_now["running"].as_bool() == Some(true) {
        let known = report_now["status"]["profiles"]
            .as_array()
            .is_some_and(|profiles| profiles.iter().any(|profile| profile.as_str() == Some(name)));
        if !known {
            return Err(format!("OpenRGB has no profile named {name}"));
        }
        // The engine does it: the request first, then the switch, so it
        // sees both together and never hands back with the old effects.
        let id = now_ms() as u64;
        write_atomic(
            &runtime_path(REQUEST_FILE)?,
            &format!("{}\n", json!({ "profile": name, "id": id })),
        )?;
        if config.active {
            config.active = false;
            save_config(&config)?;
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            thread::sleep(Duration::from_millis(150));
            let report = report();
            if report["status"]["profile_loaded"].as_u64() == Some(id) {
                return Ok(());
            }
        }
        return Err("OpenRGB did not take the profile in time; is its SDK server running?".into());
    }
    if config.active {
        config.active = false;
        save_config(&config)?;
    }
    let mut client = Client::connect(port(&config), CLIENT_NAME)?;
    if !client.profiles()?.iter().any(|profile| profile == name) {
        return Err(format!("OpenRGB has no profile named {name}"));
    }
    client.load_profile(name)
}

pub fn command(args: impl Iterator<Item = String>) -> Result<bool, String> {
    let words: Vec<String> = args.collect();
    match words.first().map(String::as_str) {
        None => {
            print_report(false);
            Ok(true)
        }
        Some("--format") if words.len() == 2 && words[1] == "json" => {
            print_report(true);
            Ok(true)
        }
        Some("run") if words.len() == 1 => run(),
        Some("live") if words.len() >= 2 => set_live(&words[1], &words[2..]).map(|()| true),
        Some("profile") if words.len() == 2 => load_profile(&words[1]).map(|()| true),
        Some(_) => {
            let config = change(load_config(), &words)?;
            save_config(&config)?;
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openrgb_sdk::tests::DescriptionWriter;

    fn words(text: &str) -> Vec<String> {
        text.split(' ').map(str::to_owned).collect()
    }

    #[test]
    fn every_theme_and_effect_is_usable() {
        for theme in THEMES {
            assert_eq!(parse_palette(theme.colors).len(), theme.colors.len(), "{}", theme.id);
        }
        let catalog = catalog();
        assert_eq!(catalog["themes"].as_array().unwrap().len(), THEMES.len() + LIVE_THEMES.len());
        assert_eq!(catalog["effects"].as_array().unwrap().len(), EFFECTS.len());
    }

    #[test]
    fn commands_change_the_configuration_and_refuse_nonsense() {
        let config = Config::default();
        assert!(!config.active);
        assert_eq!((config.theme.as_str(), config.effect.as_str()), ("synthwave", "wave"));
        let config = change(config, &words("on")).unwrap();
        assert!(config.active);
        let config = change(config, &words("theme dracula")).unwrap();
        let config = change(config, &words("effect music")).unwrap();
        let config = change(config, &words("speed 2.5")).unwrap();
        let config = change(config, &words("brightness 40")).unwrap();
        assert_eq!((config.theme.as_str(), config.effect.as_str()), ("dracula", "music"));
        assert_eq!((config.speed, config.brightness), (2.5, 40));
        assert!(change(config.clone(), &words("theme razer-chroma")).is_err());
        assert!(change(config.clone(), &words("speed 9")).is_err());
        assert!(change(config.clone(), &words("brightness 0")).is_err());
        let config = change(config, &words("rule Spotify lava stars")).unwrap();
        let config = change(config, &words("rule spotify gelo solid")).unwrap();
        assert_eq!(config.rules.len(), 1, "one rule per application");
        assert_eq!(config.rules[0].theme, "gelo");
        let config = change(config, &words("games matrix vu")).unwrap();
        assert_eq!(config.games.as_ref().unwrap().effect, "vu");
        let config = change(config, &words("device Board|I2C|X off")).unwrap();
        assert!(config.excluded.contains("Board|I2C|X"));
        let config = change(config, &words("rule Spotify remove")).unwrap();
        let config = change(config, &words("games off")).unwrap();
        assert!(config.rules.is_empty() && config.games.is_none());
    }

    #[test]
    fn a_damaged_configuration_falls_back_to_safe_values() {
        let config: Config = serde_json::from_str(
            r#"{"active": true, "theme": "nope", "effect": "boom", "speed": 40, "brightness": 0,
                "rules": [{"app": "x", "theme": "nope", "effect": "wave"}]}"#,
        )
        .unwrap();
        let config = sanitized(config);
        assert_eq!((config.theme.as_str(), config.effect.as_str()), ("synthwave", "wave"));
        assert_eq!((config.speed, config.brightness), (3.0, 5));
        assert!(config.rules.is_empty());
    }

    #[test]
    fn rules_win_over_games_which_win_over_the_theme() {
        let mut config = Config::default();
        config.rules.push(Rule {
            app: "Spotify".into(),
            theme: "gelo".into(),
            effect: "music".into(),
        });
        config.games = Some(Look {
            theme: "lava".into(),
            effect: "wave".into(),
        });
        let (look, rule) = choose_look(&config, &["spotify".into()], true);
        assert_eq!((look.theme.as_str(), rule.as_deref()), ("gelo", Some("Spotify")));
        let (look, rule) = choose_look(&config, &["Firefox".into()], true);
        assert_eq!((look.theme.as_str(), rule.as_deref()), ("lava", Some("games")));
        let (look, rule) = choose_look(&config, &[], false);
        assert_eq!((look.theme.as_str(), rule), ("synthwave", None));
    }

    #[test]
    fn games_are_told_from_other_programs() {
        assert!(is_game("Cyberpunk2077.exe", b""));
        assert!(is_game("wine64-preloader", b""));
        assert!(is_game("hl2_linux", b"HOME=/home/x\0SteamGameId=220\0"));
        assert!(!is_game("steam", b"SteamAppId=0\0"));
        assert!(!is_game("firefox", b"HOME=/home/x\0"));
    }

    #[test]
    fn leds_are_placed_along_strips_and_on_matrices() {
        let reply = DescriptionWriter::device(
            4,
            5,
            "Keyboard",
            &[("Strip", 1, 4, None), ("Keys", 2, 6, Some((2, 3))), ("Logo", 0, 1, None)],
        );
        let controller = openrgb_sdk::parse_controller(0, &reply, 4).unwrap();
        let leds = layout(&controller);
        assert_eq!(leds.len(), 11);
        assert_eq!((leds[0].along, leds[3].along), (0.0, 1.0));
        // Matrix LED 4 is row 1, column 1 of a 2x3 grid.
        assert!(leds[4 + 4].matrix);
        assert!((leds[4 + 4].x - 0.5).abs() < 1e-6 && (leds[4 + 4].y - 0.75).abs() < 1e-6);
        assert!(leds[10].single);
    }

    #[test]
    fn effects_draw_the_palette_and_follow_the_sound() {
        let palette = parse_palette(&["#ff0000", "#0000ff"]);
        let leds: Vec<Led> = (0..8)
            .map(|index| Led {
                x: index as f32 / 7.0,
                y: 0.5,
                along: index as f32 / 7.0,
                matrix: false,
                single: false,
            })
            .collect();
        let mut stars = Stars { glow: Vec::new() };
        let mut rng = Rng(7);
        let quiet = Sound {
            bands: vec![0.0; 32],
            ..Sound::default()
        };
        let solid = render("solid", &palette, &leds, 0.0, 0.0, &quiet, &mut stars, &mut rng, 0.03);
        assert_eq!(solid[0], Color { r: 255, g: 0, b: 0 });
        assert_eq!(solid[7], Color { r: 0, g: 0, b: 255 });
        let first = render("wave", &palette, &leds, 0.0, 0.0, &quiet, &mut stars, &mut rng, 0.03);
        let later = render("wave", &palette, &leds, 1.3, 0.0, &quiet, &mut stars, &mut rng, 0.03);
        assert_ne!(first, later, "a wave moves");
        let silent = render("music", &palette, &leds, 0.0, 0.0, &quiet, &mut stars, &mut rng, 0.03);
        let loud = Sound {
            bands: vec![1.0; 32],
            bass: 1.0,
            level: 1.0,
        };
        let playing = render("music", &palette, &leds, 0.0, 0.0, &loud, &mut stars, &mut rng, 0.03);
        let energy = |frame: &[Color]| -> u32 {
            frame.iter().map(|color| u32::from(color.r) + u32::from(color.g) + u32::from(color.b)).sum()
        };
        assert!(energy(&playing) > 4 * energy(&silent));
        let half = Sound {
            level: 0.5,
            ..loud.clone()
        };
        let meter = render("vu", &palette, &leds, 0.0, 0.0, &half, &mut stars, &mut rng, 0.03);
        assert!(energy(&meter[..3]) > 4 * energy(&meter[5..]), "a VU fills from the bottom");
    }

    #[test]
    fn loud_music_still_moves_the_meter_and_silence_empties_it() {
        let mut meter = Meter::new();
        // Loud music: every band between 0.65 and 0.8, with beats.
        let mut lowest = f32::MAX;
        let mut highest = f32::MIN;
        for frame in 0..300 {
            let beat = if frame % 12 < 3 { 0.8 } else { 0.65 };
            meter.listen(&[beat; 32], true);
            if frame > 100 {
                lowest = lowest.min(meter.sound.level);
                highest = highest.max(meter.sound.level);
            }
        }
        assert!(highest > 0.8 && lowest < 0.35, "{lowest}..{highest}");
        for _ in 0..60 {
            meter.listen(&[], false);
        }
        assert!(meter.sound.level < 0.01 && meter.sound.bass < 0.01);
        // A steady hum shows nothing.
        let mut steady = Meter::new();
        for _ in 0..200 {
            steady.listen(&[0.5; 32], true);
        }
        assert!(steady.sound.level < 0.05, "{}", steady.sound.level);
    }

    #[test]
    fn pastel_tones_become_vivid_on_leds_and_white_stays_white() {
        let mauve = vivid(Color::parse("#cba6f7").unwrap());
        assert!(mauve.b == 255 && mauve.g < 150, "{mauve:?}");
        assert_eq!(vivid(Color::parse("#f0f0f0").unwrap()), Color { r: 255, g: 255, b: 255 });
    }

    #[test]
    fn the_headset_gets_its_own_effect_and_ten_colors() {
        assert_eq!(headset_effect("rainbow"), "wave");
        assert_eq!(headset_effect("music"), "solid");
        assert_eq!(headset_speed(2.4), "2.0");
        let colors = headset_colors("solid", &parse_palette(&["#ff0000"]), 0.5);
        assert_eq!(colors.len(), 10);
        assert!(colors.iter().all(|color| color == "#800000"));
    }

    #[test]
    fn a_headset_changed_from_outside_is_noticed() {
        let sent = (vec!["#001a07".to_owned(); 10], "solid".to_owned(), "1.0".to_owned());
        let mut state = crate::state::RuntimeState {
            logo_colors: Some(vec!["#001A07".into(); 5]),
            ring_colors: Some(vec!["#001a07".into(); 5]),
            logo_effect: Some("solid".into()),
            ..Default::default()
        };
        assert!(!headset_differs(&state, &sent), "as sent (any letter case)");
        state.ring_colors = Some(vec!["#ff0000".into(); 5]);
        assert!(headset_differs(&state, &sent), "a color picked on the lighting card");
        state.ring_colors = Some(vec!["#001a07".into(); 5]);
        state.logo_effect = Some("glitch".into());
        assert!(headset_differs(&state, &sent), "another effect");
        state.logo_effect = None;
        state.logo_colors = None;
        assert!(!headset_differs(&state, &sent), "nothing known is not a change");
    }

    #[test]
    fn a_flash_pulses_then_ends() {
        let flash = Flash {
            color: Color::BLACK,
            started: Instant::now() - Duration::from_millis(210),
            pulses: 1,
        };
        assert!(flash.amount().unwrap() > 0.9);
        let over = Flash {
            started: Instant::now() - Duration::from_secs(1),
            ..flash
        };
        assert!(over.amount().is_none());
    }
}
