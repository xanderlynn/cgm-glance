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
    Primary,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum IconStyle {
    #[default]
    Number,
    Dot,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct TraySettings {
    pub mode: TrayMode,
    pub primary: Option<String>,
    pub icon_style: IconStyle,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BadgeStyle {
    Dot,
    #[default]
    Number,
    Arrow,
    Age,
    Card,
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
}

impl Default for BadgeSettings {
    fn default() -> Self {
        BadgeSettings {
            enabled: false,
            people: vec![],
            combined: false,
            style: BadgeStyle::Number,
            size: BadgeSize::M,
            layer: Layer::Top,
            position: PositionMode::Free,
            edge: Edge::Right,
            corner: Corner::TopRight,
            lock: false,
            click_through: false,
            opacity: 1.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Settings {
    pub tray: TraySettings,
    pub badge: BadgeSettings,
    /// Demo sources produce a new reading every 10 s instead of every 5 min.
    pub demo_fast: bool,
    pub autostart: bool,
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

pub fn load(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("cgm-glance: config unreadable ({e}); starting fresh");
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}

/// Write to a temp file then rename, so a crash never leaves a half-written config.
pub fn save(path: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(cfg)?)?;
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
    fn tolerates_old_or_partial_files() {
        let c: Config = serde_json::from_str(r#"{"people":[]}"#).unwrap();
        assert!(!c.settings.badge.enabled);
    }
}
