//! Finds the Plasma desktop wallpaper so the widget's "Vidro" background can
//! show it blurred behind the popup. Read-only: it only parses
//! `plasma-org.kde.plasma.desktop-appletsrc` and looks at the image files.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The wallpaper image of the desktop on `preferred_screen` (falling back to
/// any desktop), from the contents of `plasma-org.kde.plasma.desktop-appletsrc`.
/// Returns the raw `Image=` value, which may be a `file://` URL or a path.
pub fn configured_image(appletsrc: &str, preferred_screen: u32) -> Option<String> {
    // group -> key -> value
    let mut groups: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current = String::new();
    for line in appletsrc.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            current = line.to_owned();
        } else if let Some((key, value)) = line.split_once('=') {
            groups
                .entry(current.clone())
                .or_default()
                .insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }

    // Desktop containments (not panels) with their screen and wallpaper plugin.
    let mut desktops = groups
        .iter()
        .filter_map(|(group, keys)| {
            let id = group.strip_prefix("[Containments][")?.strip_suffix(']')?;
            if id.contains(']') || keys.get("plugin").map(String::as_str) == Some("org.kde.panel") {
                return None;
            }
            let plugin = keys.get("wallpaperplugin")?;
            let screen = keys.get("lastScreen")?.parse::<u32>().ok()?;
            Some((screen, id.to_owned(), plugin.to_owned()))
        })
        .collect::<Vec<_>>();
    desktops.sort_by_key(|(screen, id, _)| (*screen != preferred_screen, *screen, id.clone()));

    desktops.into_iter().find_map(|(_, id, plugin)| {
        groups
            .get(&format!(
                "[Containments][{id}][Wallpaper][{plugin}][General]"
            ))?
            .get("Image")
            .filter(|image| !image.is_empty())
            .cloned()
    })
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp"
            )
        })
}

/// Turns an `Image=` value into an image file. A wallpaper package (a
/// directory) resolves to its largest image in `contents/images/`.
pub fn resolve(image: &str) -> Option<PathBuf> {
    let path = PathBuf::from(image.strip_prefix("file://").unwrap_or(image));
    if path.is_file() && is_image(&path) {
        return Some(path);
    }
    if path.is_dir() {
        let images = path.join("contents").join("images");
        return fs::read_dir(images)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|candidate| candidate.is_file() && is_image(candidate))
            .max_by_key(|candidate| fs::metadata(candidate).map(|meta| meta.len()).unwrap_or(0));
    }
    None
}

/// Plasma's own default wallpaper, used when none is configured.
const DEFAULT_PACKAGE: &str = "/usr/share/wallpapers/Next";

/// `soniccore wallpaper --format json`
pub fn print_json() -> Result<(), String> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("HOME is not set")?;
    let appletsrc = fs::read_to_string(config_home.join("plasma-org.kde.plasma.desktop-appletsrc"))
        .unwrap_or_default();
    let image = configured_image(&appletsrc, 0)
        .and_then(|image| resolve(&image))
        .or_else(|| resolve(DEFAULT_PACKAGE))
        .ok_or("no Plasma wallpaper image was found")?;
    println!(
        "{}",
        serde_json::json!({ "schema": 1, "path": image.to_string_lossy() })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPLETSRC: &str = "\
[Containments][361]
lastScreen=0
plugin=org.kde.plasma.folder
wallpaperplugin=org.kde.slideshow

[Containments][361][Wallpaper][org.kde.slideshow][General]
Image=file:///wallpapers/space.png
SlidePaths=/wallpapers/

[Containments][362]
lastScreen=1
plugin=org.kde.plasma.folder
wallpaperplugin=org.kde.image

[Containments][362][Wallpaper][org.kde.image][General]
Image=/wallpapers/second.jpg

[Containments][363]
lastScreen=0
plugin=org.kde.panel
wallpaperplugin=org.kde.image

[Containments][363][Wallpaper][org.kde.image][General]
Image=/panel/never.png
";

    #[test]
    fn the_preferred_screen_desktop_wins_and_panels_are_ignored() {
        assert_eq!(
            configured_image(APPLETSRC, 0).as_deref(),
            Some("file:///wallpapers/space.png")
        );
        assert_eq!(
            configured_image(APPLETSRC, 1).as_deref(),
            Some("/wallpapers/second.jpg")
        );
        // An unknown screen falls back to the lowest screen with an image.
        assert_eq!(
            configured_image(APPLETSRC, 7).as_deref(),
            Some("file:///wallpapers/space.png")
        );
        assert_eq!(configured_image("", 0), None);
    }

    #[test]
    fn packages_resolve_to_their_largest_image() {
        let root = std::env::temp_dir().join(format!("soniccore-wallpaper-{}", std::process::id()));
        let images = root.join("Package/contents/images");
        fs::create_dir_all(&images).unwrap();
        fs::write(images.join("1920x1080.png"), vec![0; 10]).unwrap();
        fs::write(images.join("3840x2160.png"), vec![0; 40]).unwrap();
        fs::write(images.join("notes.txt"), vec![0; 99]).unwrap();
        let single = root.join("single.JPG");
        fs::write(&single, b"x").unwrap();

        assert_eq!(
            resolve(&format!("file://{}", root.join("Package").display())),
            Some(images.join("3840x2160.png"))
        );
        assert_eq!(resolve(single.to_str().unwrap()), Some(single.clone()));
        assert_eq!(resolve(root.join("missing.png").to_str().unwrap()), None);
        fs::remove_dir_all(root).unwrap();
    }
}
