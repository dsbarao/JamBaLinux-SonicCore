//! Finds the desktop wallpaper so the app's "Vidro" and "Papel de parede"
//! backgrounds can show it, on Plasma, GNOME (and Ubuntu, Pop!_OS 22.04),
//! COSMIC, Cinnamon, MATE and Xfce. Read-only: it parses each desktop's own
//! setting (a config file or `gsettings`/`xfconf-query`) and looks at the
//! image files.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
                "png" | "jpg" | "jpeg" | "webp" | "svg" | "jxl" | "avif" | "bmp"
            )
        })
}

/// The value of a `gsettings get` answer: `'file:///a b.jpg'` → `file:///a b.jpg`
/// (an empty or unset value is `None`).
pub fn gsettings_string(answer: &str) -> Option<String> {
    let value = answer.trim();
    let value = value
        .strip_prefix('\'')
        .and_then(|inner| inner.strip_suffix('\''))
        .unwrap_or(value);
    (!value.is_empty()).then(|| value.replace("\\'", "'"))
}

/// A GNOME `file://` URI with percent escapes (`%20`) as a path.
fn uri_path(uri: &str) -> String {
    let raw = uri.strip_prefix("file://").unwrap_or(uri);
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&raw[index + 1..index + 3], 16)
        {
            decoded.push(byte);
            index += 3;
            continue;
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// The first image of a GNOME slideshow (`.xml`) definition.
pub fn slideshow_first_file(xml: &str) -> Option<String> {
    let start = xml.find("<file>")? + "<file>".len();
    let end = start + xml[start..].find("</file>")?;
    let file = xml[start..end].trim();
    (!file.is_empty()).then(|| file.to_owned())
}

/// COSMIC's background setting (RON): the image path of `source: Path("…")`.
pub fn cosmic_source(ron: &str) -> Option<String> {
    let start = ron.find("Path(\"")? + "Path(\"".len();
    let end = start + ron[start..].find('"')?;
    let path = &ron[start..end];
    (!path.is_empty()).then(|| path.to_owned())
}

/// Which desktop runs, from `XDG_CURRENT_DESKTOP` ("ubuntu:GNOME", "KDE", …).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desktop {
    Plasma,
    Gnome,
    Cosmic,
    Cinnamon,
    Mate,
    Xfce,
    Other,
}

pub fn desktop_from(current: &str) -> Desktop {
    let names = current
        .split(':')
        .map(|name| name.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let has = |wanted: &str| names.iter().any(|name| name == wanted);
    if has("kde") {
        Desktop::Plasma
    } else if has("cosmic") {
        Desktop::Cosmic
    } else if has("x-cinnamon") || has("cinnamon") {
        Desktop::Cinnamon
    } else if has("mate") {
        Desktop::Mate
    } else if has("xfce") {
        Desktop::Xfce
    } else if has("gnome") || has("unity") || has("pop") {
        Desktop::Gnome
    } else {
        Desktop::Other
    }
}

/// Turns an `Image=` value into an image file. A wallpaper package (a
/// directory) resolves to its largest image in `contents/images/`.
pub fn resolve(image: &str) -> Option<PathBuf> {
    let path = PathBuf::from(image.strip_prefix("file://").unwrap_or(image));
    if path.is_file() && is_image(&path) {
        return Some(path);
    }
    if path.is_file() && path.extension().is_some_and(|extension| extension == "xml") {
        let xml = fs::read_to_string(&path).ok()?;
        return resolve(&slideshow_first_file(&xml)?);
    }
    if path.is_dir() {
        // A Plasma wallpaper package: its largest image.
        let images = path.join("contents").join("images");
        if images.is_dir() {
            return fs::read_dir(images)
                .ok()?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|candidate| candidate.is_file() && is_image(candidate))
                .max_by_key(|candidate| fs::metadata(candidate).map(|meta| meta.len()).unwrap_or(0));
        }
        // A slideshow folder (COSMIC, Xfce): its first image by name.
        let mut images = fs::read_dir(&path)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|candidate| candidate.is_file() && is_image(candidate))
            .collect::<Vec<_>>();
        images.sort();
        return images.into_iter().next();
    }
    None
}

fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn gsettings(schema: &str, key: &str) -> Option<String> {
    gsettings_string(&command_output("gsettings", &["get", schema, key])?)
}

fn plasma_image(config_home: &Path) -> Option<PathBuf> {
    let appletsrc =
        fs::read_to_string(config_home.join("plasma-org.kde.plasma.desktop-appletsrc")).ok()?;
    resolve(&configured_image(&appletsrc, 0)?)
}

fn gnome_image() -> Option<PathBuf> {
    // The dark variant while the desktop is in dark mode.
    let dark = gsettings("org.gnome.desktop.interface", "color-scheme")
        .is_some_and(|scheme| scheme == "prefer-dark");
    let keys: &[&str] = if dark {
        &["picture-uri-dark", "picture-uri"]
    } else {
        &["picture-uri", "picture-uri-dark"]
    };
    keys.iter()
        .filter_map(|key| gsettings("org.gnome.desktop.background", key))
        .find_map(|uri| resolve(&uri_path(&uri)))
}

fn cinnamon_image() -> Option<PathBuf> {
    resolve(&uri_path(&gsettings("org.cinnamon.desktop.background", "picture-uri")?))
}

fn mate_image() -> Option<PathBuf> {
    resolve(&gsettings("org.mate.background", "picture-filename")?)
}

fn cosmic_image(config_home: &Path) -> Option<PathBuf> {
    let directory = config_home.join("cosmic/com.system76.CosmicBackground/v1");
    // "all" is the image of every display; otherwise the first display's.
    let mut files = fs::read_dir(&directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort_by_key(|path| path.file_name().is_none_or(|name| name != "all"));
    files
        .iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .find_map(|ron| resolve(&cosmic_source(&ron)?))
}

fn xfce_image() -> Option<PathBuf> {
    let properties = command_output("xfconf-query", &["-c", "xfce4-desktop", "-l"])?;
    let mut keys = properties
        .lines()
        .filter(|key| key.ends_with("/last-image"))
        .collect::<Vec<_>>();
    keys.sort();
    keys.into_iter().find_map(|key| {
        let value = command_output("xfconf-query", &["-c", "xfce4-desktop", "-p", key])?;
        resolve(value.trim())
    })
}

/// The wallpaper of the running desktop, then of any other desktop found.
fn desktop_image(desktop: Desktop, config_home: &Path) -> Option<PathBuf> {
    let first = match desktop {
        Desktop::Plasma => plasma_image(config_home),
        Desktop::Gnome => gnome_image(),
        Desktop::Cosmic => cosmic_image(config_home),
        Desktop::Cinnamon => cinnamon_image(),
        Desktop::Mate => mate_image(),
        Desktop::Xfce => xfce_image(),
        Desktop::Other => None,
    };
    first
        .or_else(|| plasma_image(config_home))
        .or_else(|| cosmic_image(config_home))
        .or_else(gnome_image)
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
    let desktop = desktop_from(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default());
    let image = desktop_image(desktop, &config_home)
        .or_else(|| resolve(DEFAULT_PACKAGE))
        .ok_or("no desktop wallpaper image was found")?;
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
    fn each_desktop_is_recognized_from_xdg_current_desktop() {
        assert_eq!(desktop_from("KDE"), Desktop::Plasma);
        assert_eq!(desktop_from("ubuntu:GNOME"), Desktop::Gnome);
        assert_eq!(desktop_from("pop:GNOME"), Desktop::Gnome);
        assert_eq!(desktop_from("COSMIC"), Desktop::Cosmic);
        assert_eq!(desktop_from("X-Cinnamon"), Desktop::Cinnamon);
        assert_eq!(desktop_from("MATE"), Desktop::Mate);
        assert_eq!(desktop_from("XFCE"), Desktop::Xfce);
        assert_eq!(desktop_from("sway"), Desktop::Other);
        assert_eq!(desktop_from(""), Desktop::Other);
    }

    #[test]
    fn gnome_cinnamon_and_cosmic_settings_are_read() {
        assert_eq!(
            gsettings_string("'file:///usr/share/backgrounds/gnome/blobs-d.svg'\n").as_deref(),
            Some("file:///usr/share/backgrounds/gnome/blobs-d.svg")
        );
        assert_eq!(gsettings_string("''"), None);
        assert_eq!(uri_path("file:///home/a/My%20Pictures/sea.jpg"), "/home/a/My Pictures/sea.jpg");
        assert_eq!(uri_path("/plain/path.png"), "/plain/path.png");
        assert_eq!(
            slideshow_first_file("<background><static><duration>1</duration><file>/a/day.jpg</file></static></background>")
                .as_deref(),
            Some("/a/day.jpg")
        );
        let ron = "(\n    output: \"all\",\n    source: Path(\"/usr/share/backgrounds/pop/kate.png\"),\n    filter_by_theme: true,\n)";
        assert_eq!(cosmic_source(ron).as_deref(), Some("/usr/share/backgrounds/pop/kate.png"));
        assert_eq!(cosmic_source("(source: Color(Single((0.1, 0.2, 0.3))))"), None);
    }

    #[test]
    fn slideshow_folders_and_xml_resolve_to_an_image() {
        let root = std::env::temp_dir().join(format!("soniccore-slides-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("b.jpg"), b"x").unwrap();
        fs::write(root.join("a.png"), b"x").unwrap();
        fs::write(root.join("readme.txt"), b"x").unwrap();
        assert_eq!(resolve(root.to_str().unwrap()), Some(root.join("a.png")));
        let xml = root.join("show.xml");
        fs::write(&xml, format!("<background><static><file>{}</file></static></background>", root.join("b.jpg").display())).unwrap();
        assert_eq!(resolve(xml.to_str().unwrap()), Some(root.join("b.jpg")));
        fs::remove_dir_all(root).unwrap();
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
