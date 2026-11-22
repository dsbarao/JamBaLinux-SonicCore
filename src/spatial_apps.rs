//! The per-application 3D choice ("3D por aplicativo").
//!
//! With the spatial sound on, every application the equalizer routes also
//! goes through the 3D graph. The user can turn 3D off for single
//! applications (a music player, a browser): those stay on the equalizer
//! only. The choice is remembered by application name in
//! `spatial-apps.json`, and the equalizer's routing loop applies it to every
//! stream of that application, now and when it plays again.

use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "spatial-apps.json";
/// Names longer than this are not application names.
const MAX_NAME_LEN: usize = 128;
/// Enough for every application a person uses.
const MAX_APPS: usize = 200;

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
struct SpatialApps {
    #[serde(default)]
    schema: u8,
    /// Applications whose 3D is off.
    #[serde(default)]
    flat: BTreeSet<String>,
}

fn path() -> Result<PathBuf, String> {
    Ok(crate::equalizer::config_directory()?.join(FILE_NAME))
}

fn load() -> SpatialApps {
    path()
        .ok()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save(apps: &SpatialApps) -> Result<(), String> {
    let path = path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(apps).map_err(|error| error.to_string())?;
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

/// Applications whose 3D is off; empty when nothing was chosen.
pub fn flat_apps() -> HashSet<String> {
    load().flat.into_iter().collect()
}

fn validated_name(app: &str) -> Result<&str, String> {
    let app = app.trim();
    if app.is_empty() || app.len() > MAX_NAME_LEN || app.chars().any(char::is_control) {
        return Err("invalid application name".into());
    }
    Ok(app)
}

fn with_choice(mut apps: SpatialApps, app: &str, enabled: bool) -> Result<SpatialApps, String> {
    let app = validated_name(app)?;
    if enabled {
        apps.flat.remove(app);
    } else {
        if apps.flat.len() >= MAX_APPS && !apps.flat.contains(app) {
            return Err("too many applications without 3D".into());
        }
        apps.flat.insert(app.to_owned());
    }
    apps.schema = 1;
    Ok(apps)
}

/// Turns 3D on or off for `app`; the routing loop moves its streams within
/// half a second.
pub fn set_app_3d(app: &str, enabled: bool) -> Result<(), String> {
    save(&with_choice(load(), app, enabled)?)
}

/// One application playing now, for the list in the spatial card.
#[derive(Debug, Serialize, PartialEq)]
pub struct AppEntry {
    pub app: String,
    pub icon: Option<String>,
    /// 3D is on for it (the default).
    pub spatial: bool,
    /// Where it plays now: `spatial`, `equalizer` or `other`.
    pub route: &'static str,
}

/// The applications playing now (one entry each), with their 3D choice.
pub fn playing_apps() -> Result<Vec<AppEntry>, String> {
    let flat = flat_apps();
    let mut entries: Vec<AppEntry> = Vec::new();
    for stream in crate::pipewire::playing_streams()? {
        if let Some(entry) = entries.iter_mut().find(|entry| entry.app == stream.app) {
            // Any stream of the app in 3D shows the app in 3D.
            if stream.route == "spatial" || entry.route == "other" {
                entry.route = stream.route;
            }
            continue;
        }
        entries.push(AppEntry {
            spatial: !flat.contains(&stream.app),
            app: stream.app,
            icon: stream.icon,
            route: stream.route,
        });
    }
    entries.sort_by_key(|entry| entry.app.to_lowercase());
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_3d_off_and_on_records_only_the_flat_applications() {
        let apps = with_choice(SpatialApps::default(), " Spotify ", false).unwrap();
        assert!(apps.flat.contains("Spotify"));
        let apps = with_choice(apps, "Spotify", true).unwrap();
        assert!(apps.flat.is_empty());
        assert!(with_choice(SpatialApps::default(), "", false).is_err());
        assert!(with_choice(SpatialApps::default(), "a\nb", false).is_err());
        assert!(with_choice(SpatialApps::default(), &"x".repeat(200), false).is_err());
    }

    #[test]
    fn a_missing_or_old_file_means_3d_for_everything() {
        let apps: SpatialApps = serde_json::from_str("{}").unwrap();
        assert!(apps.flat.is_empty());
    }
}
