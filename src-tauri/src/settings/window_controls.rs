//! The desktop's own window buttons: which look they have and on which side each one sits, so
//! borderless mode can draw controls that match the frames of every other window.

// The KDE and GNOME parsers only run on Linux, but stay compiled everywhere for their tests.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use serde::Serialize;

use super::WindowControlsStyle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum WindowButton {
    Minimize,
    Maximize,
    Close,
}

/// What the running desktop uses. `style` is never [`WindowControlsStyle::Auto`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SystemWindowControls {
    pub style: WindowControlsStyle,
    pub left: Vec<WindowButton>,
    pub right: Vec<WindowButton>,
}

impl SystemWindowControls {
    fn right(style: WindowControlsStyle) -> Self {
        Self {
            style,
            left: Vec::new(),
            right: vec![
                WindowButton::Minimize,
                WindowButton::Maximize,
                WindowButton::Close,
            ],
        }
    }
}

/// Detected once: the desktop and its button layout only change across a restart in practice,
/// and GNOME's layout costs a `gsettings` call.
pub fn system() -> SystemWindowControls {
    static SYSTEM: std::sync::OnceLock<SystemWindowControls> = std::sync::OnceLock::new();
    SYSTEM.get_or_init(detect).clone()
}

#[cfg(target_os = "linux")]
fn detect() -> SystemWindowControls {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if desktop
        .split(':')
        .any(|name| name.eq_ignore_ascii_case("KDE"))
    {
        return kde_layout(&kde_config_files());
    }
    let layout = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.wm.preferences", "button-layout"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned());
    match layout.as_deref().and_then(gnome_layout) {
        Some(controls) => controls,
        None => SystemWindowControls::right(WindowControlsStyle::Adwaita),
    }
}

#[cfg(target_os = "windows")]
fn detect() -> SystemWindowControls {
    SystemWindowControls::right(WindowControlsStyle::Windows)
}

#[cfg(target_os = "macos")]
fn detect() -> SystemWindowControls {
    SystemWindowControls {
        style: WindowControlsStyle::Macos,
        left: vec![
            WindowButton::Close,
            WindowButton::Minimize,
            WindowButton::Maximize,
        ],
        right: Vec::new(),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn detect() -> SystemWindowControls {
    SystemWindowControls::right(WindowControlsStyle::Minimal)
}

/// `kwinrc` in KDE's lookup order, most specific first: the user's file, the defaults a
/// Global Theme wrote, then the system-wide file.
#[cfg(target_os = "linux")]
fn kde_config_files() -> Vec<String> {
    let user = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".config"))
        });
    let mut paths = Vec::new();
    if let Some(user) = user {
        paths.push(user.join("kwinrc"));
        paths.push(user.join("kdedefaults").join("kwinrc"));
    }
    paths.push(std::path::PathBuf::from("/etc/xdg/kwinrc"));
    paths
        .into_iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect()
}

const KDE_GROUP: &str = "[org.kde.kdecoration2]";
// KWin's defaults when no file sets them: window menu and on-all-desktops on the left,
// context help, minimize, maximize and close on the right.
const KDE_DEFAULT_LEFT: &str = "MS";
const KDE_DEFAULT_RIGHT: &str = "HIAX";

/// Breeze with the button sides from the first `kwinrc` that sets them.
fn kde_layout(files: &[String]) -> SystemWindowControls {
    let left = files
        .iter()
        .find_map(|file| ini_value(file, KDE_GROUP, "ButtonsOnLeft"));
    let right = files
        .iter()
        .find_map(|file| ini_value(file, KDE_GROUP, "ButtonsOnRight"));
    SystemWindowControls {
        style: WindowControlsStyle::Breeze,
        left: kde_buttons(left.unwrap_or(KDE_DEFAULT_LEFT)),
        right: kde_buttons(right.unwrap_or(KDE_DEFAULT_RIGHT)),
    }
}

/// KWin spells a button row as letters; only the three this app can act on are kept.
fn kde_buttons(row: &str) -> Vec<WindowButton> {
    row.chars()
        .filter_map(|letter| match letter {
            'I' => Some(WindowButton::Minimize),
            'A' => Some(WindowButton::Maximize),
            'X' => Some(WindowButton::Close),
            _ => None,
        })
        .collect()
}

fn ini_value<'a>(file: &'a str, group: &str, key: &str) -> Option<&'a str> {
    let mut in_group = false;
    for line in file.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == group;
            continue;
        }
        if !in_group {
            continue;
        }
        if let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            return Some(value.trim());
        }
    }
    None
}

/// GNOME's `button-layout`, printed by `gsettings` as `'appmenu:minimize,maximize,close'`:
/// buttons left of the colon go on the left.
fn gnome_layout(value: &str) -> Option<SystemWindowControls> {
    let value = value.trim().trim_matches('\'');
    let (left, right) = value.split_once(':')?;
    let buttons = |side: &str| {
        side.split(',')
            .filter_map(|name| match name.trim() {
                "minimize" => Some(WindowButton::Minimize),
                "maximize" => Some(WindowButton::Maximize),
                "close" => Some(WindowButton::Close),
                _ => None,
            })
            .collect()
    };
    Some(SystemWindowControls {
        style: WindowControlsStyle::Adwaita,
        left: buttons(left),
        right: buttons(right),
    })
}

#[cfg(test)]
mod tests {
    use super::WindowButton::{Close, Maximize, Minimize};
    use super::*;

    #[test]
    fn kde_defaults_put_minimize_maximize_close_on_the_right() {
        let controls = kde_layout(&[]);
        assert_eq!(controls.style, WindowControlsStyle::Breeze);
        assert!(controls.left.is_empty());
        assert_eq!(controls.right, vec![Minimize, Maximize, Close]);
    }

    #[test]
    fn kde_layout_follows_the_most_specific_file() {
        let user = "[General]\nfoo=1\n[org.kde.kdecoration2]\nButtonsOnLeft=XIA\nButtonsOnRight=\n";
        let theme = "[org.kde.kdecoration2]\nButtonsOnLeft=M\nButtonsOnRight=IAX\n";
        let controls = kde_layout(&[user.into(), theme.into()]);
        assert_eq!(controls.left, vec![Close, Minimize, Maximize]);
        assert!(controls.right.is_empty());
    }

    #[test]
    fn kde_keys_outside_the_decoration_group_are_ignored() {
        let file = "[Windows]\nButtonsOnRight=X\n[org.kde.kdecoration2]\nlibrary=org.kde.breeze\n";
        let controls = kde_layout(&[file.into()]);
        assert_eq!(controls.right, vec![Minimize, Maximize, Close]);
    }

    #[test]
    fn gnome_layout_splits_on_the_colon() {
        let controls = gnome_layout("'close,minimize:appmenu'\n").expect("layout");
        assert_eq!(controls.left, vec![Close, Minimize]);
        assert!(controls.right.is_empty());
        let controls = gnome_layout("'appmenu:close'").expect("layout");
        assert_eq!(controls.right, vec![Close]);
        assert!(gnome_layout("nonsense").is_none());
    }
}
