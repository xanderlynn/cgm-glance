//! Non-secret settings, saved as JSON in the OS config folder.
//!
//! By design no type in this file can hold a password, token or username.
//! Those live only in the OS keychain (see `secrets.rs`).

use crate::model::{Thresholds, Units};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    Us,
    Ous,
    Jp,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    /// A realistic day: drifts through range, sometimes high or low.
    Normal,
    Steady,
    High,
    Low,
    UrgentLow,
    /// Last reading 14 minutes old.
    Stale,
    /// Last reading 35 minutes old.
    NoData,
    /// Simulates a rejected password.
    SignIn,
    /// Simulates no internet.
    Offline,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceConfig {
    DexcomShare { region: Region },
    Nightscout { url: String },
    Demo { scenario: Scenario },
}

impl SourceConfig {
    pub fn label(&self) -> String {
        match self {
            SourceConfig::DexcomShare { region } => match region {
                Region::Us => "Dexcom Share (US)".into(),
                Region::Ous => "Dexcom Share (outside US)".into(),
                Region::Jp => "Dexcom Share (Japan)".into(),
            },
            SourceConfig::Nightscout { .. } => "Nightscout".into(),
            SourceConfig::Demo { .. } => "Demo data".into(),
        }
    }
    pub fn is_demo(&self) -> bool {
        matches!(self, SourceConfig::Demo { .. })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Person {
    /// Random id. Also the keychain entry name.
    pub id: String,
    pub name: String,
    pub initials: String,
    pub color: String,
    #[serde(default)]
    pub units: Units,
    #[serde(default)]
    pub thresholds: Thresholds,
    pub source: SourceConfig,
    #[serde(default = "yes")]
    pub tray_icon: bool,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrayMode {
    #[default]
    PerPerson,
    /// One icon showing whoever needs attention most.
    Primary,
    /// One icon showing only the chosen person (`TraySettings::primary`).
    Single,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum IconStyle {
    #[default]
    Number,
    Dot,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TraySettings {
    pub mode: TrayMode,
    pub primary: Option<String>,
    pub icon_style: IconStyle,
    /// Initials beside the number in the menu-bar title.
    pub show_initials: bool,
}

impl Default for TraySettings {
    fn default() -> Self {
        TraySettings {
            mode: TrayMode::default(),
            primary: None,
            icon_style: IconStyle::default(),
            show_initials: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BadgeStyle {
    /// Older versions had more styles; they all open as Pill so no setup is lost.
    #[default]
    #[serde(
        alias = "dot",
        alias = "number",
        alias = "arrow",
        alias = "age",
        alias = "card"
    )]
    Pill,
    Ring,
    /// Emoji face instead of the number. The number shows on hover and in the tray.
    Faces,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BadgeSize {
    S,
    #[default]
    M,
    L,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    #[default]
    Top,
    Normal,
    Bottom,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PositionMode {
    #[default]
    Free,
    Edge,
    Corner,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Left,
    #[default]
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BadgeSettings {
    pub enabled: bool,
    /// Empty means everyone.
    pub people: Vec<String>,
    pub combined: bool,
    pub style: BadgeStyle,
    pub size: BadgeSize,
    pub layer: Layer,
    pub position: PositionMode,
    pub edge: Edge,
    pub corner: Corner,
    pub lock: bool,
    pub click_through: bool,
    pub opacity: f64,
    /// Initials on each badge when there are several people.
    pub show_initials: bool,
}

impl Default for BadgeSettings {
    fn default() -> Self {
        BadgeSettings {
            enabled: false,
            people: vec![],
            combined: false,
            style: BadgeStyle::Pill,
            size: BadgeSize::M,
            layer: Layer::Top,
            position: PositionMode::Free,
            edge: Edge::Right,
            corner: Corner::TopRight,
            lock: false,
            click_through: false,
            opacity: 1.0,
            show_initials: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub tray: TraySettings,
    pub badge: BadgeSettings,
    pub alarms: crate::alarms::AlarmSettings,
    /// Demo sources produce a new reading every 10 s instead of every 5 min.
    pub demo_fast: bool,
    pub autostart: bool,
    /// Check GitHub Releases for a new version shortly after start and then daily.
    pub auto_update: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            tray: TraySettings::default(),
            badge: BadgeSettings::default(),
            alarms: Default::default(),
            demo_fast: false,
            autostart: false,
            auto_update: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedPos {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub people: Vec<Person>,
    pub settings: Settings,
    pub badge_positions: HashMap<String, SavedPos>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: 1,
            people: vec![],
            settings: Settings::default(),
            badge_positions: HashMap::new(),
        }
    }
}

/// `config.json`, or `config.demo.json` in demo mode so demo runs never touch real settings.
pub fn file_path(dir: &Path, demo: bool) -> PathBuf {
    dir.join(if demo {
        "config.demo.json"
    } else {
        "config.json"
    })
}

pub fn is_color(c: &str) -> bool {
    let b = c.as_bytes();
    b.len() == 7 && b[0] == b'#' && b[1..].iter().all(|x| x.is_ascii_hexdigit())
}

/// Makes the settings folder and file private to the current user (no-op elsewhere).
fn make_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(dir) = path.parent().filter(|d| d.exists()) {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        if path.exists() {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

pub fn load(path: &Path) -> Config {
    // Files saved by earlier versions may be readable by other users; fix that first.
    make_private(path);
    let mut cfg = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            // Keep the unreadable file instead of overwriting it on the next save.
            let backup = path.with_extension("json.unreadable");
            let _ = std::fs::rename(path, &backup);
            eprintln!(
                "cgm-glance: settings unreadable ({e}); kept a copy at {}",
                backup.display()
            );
            Config::default()
        }),
        Err(_) => Config::default(),
    };
    // Colors go into page styles, so only plain #RRGGBB values are allowed.
    // Hand-edited or older files: keep alarm rules in range.
    cfg.settings.alarms.normalize();
    for p in &mut cfg.people {
        if !is_color(&p.color) {
            p.color = "#4C6FFF".into();
        }
    }
    cfg
}

/// Write to a temp file then rename, so a crash never leaves a half-written config.
/// The file is readable only by the current user (names and site addresses are personal).
pub fn save(path: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(cfg)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Config {
        let mut c = Config::default();
        c.people.push(Person {
            id: "abc".into(),
            name: "Alex".into(),
            initials: "AL".into(),
            color: "#4C6FFF".into(),
            units: Units::Mgdl,
            thresholds: Thresholds::default(),
            source: SourceConfig::DexcomShare { region: Region::Us },
            tray_icon: true,
        });
        c.people.push(Person {
            id: "def".into(),
            name: "Mia".into(),
            initials: "MJ".into(),
            color: "#E0559B".into(),
            units: Units::Mmol,
            thresholds: Thresholds::default(),
            source: SourceConfig::Nightscout {
                url: "https://example.org".into(),
            },
            tray_icon: true,
        });
        c
    }

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("cgm-glance-test-{}", uuid::Uuid::new_v4()));
        let path = file_path(&dir, false);
        let c = sample();
        save(&path, &c).unwrap();
        assert_eq!(load(&path), c);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn config_has_no_secret_fields() {
        let json = serde_json::to_string(&sample()).unwrap().to_lowercase();
        for word in ["password", "token", "username", "secret", "session"] {
            assert!(
                !json.contains(word),
                "config JSON must never contain '{word}'"
            );
        }
    }

    #[test]
    fn old_badge_styles_become_pill() {
        for old in ["dot", "number", "arrow", "age", "card"] {
            let c: Config = serde_json::from_str(&format!(
                r#"{{"settings":{{"badge":{{"style":"{old}"}}}}}}"#
            ))
            .unwrap();
            assert_eq!(c.settings.badge.style, BadgeStyle::Pill);
        }
    }

    #[test]
    #[cfg(unix)]
    fn saved_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("cgm-glance-test-{}", uuid::Uuid::new_v4()));
        let path = file_path(&dir, false);
        save(&path, &sample()).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn bad_colors_are_replaced_and_unreadable_files_kept() {
        let dir = std::env::temp_dir().join(format!("cgm-glance-test-{}", uuid::Uuid::new_v4()));
        let path = file_path(&dir, false);
        let mut c = sample();
        c.people[0].color = "red;background:url(x)".into();
        save(&path, &c).unwrap();
        assert_eq!(load(&path).people[0].color, "#4C6FFF");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(load(&path).people.is_empty());
        assert!(path.with_extension("json.unreadable").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tolerates_old_or_partial_files() {
        let c: Config = serde_json::from_str(r#"{"people":[]}"#).unwrap();
        assert!(!c.settings.badge.enabled);
    }
}
