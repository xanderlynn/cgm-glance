//! Core glucose types and the display rules (ranges, staleness, formatting).

use serde::{Deserialize, Serialize};

pub const STALE_MIN: i64 = 10;
pub const NO_DATA_MIN: i64 = 20;
pub const MMOL_FACTOR: f64 = 18.0182;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Trend {
    DoubleUp,
    SingleUp,
    FortyFiveUp,
    Flat,
    FortyFiveDown,
    SingleDown,
    DoubleDown,
    Unknown,
}

impl Trend {
    pub fn arrow(self) -> &'static str {
        match self {
            Trend::DoubleUp => "⇈",
            Trend::SingleUp => "↑",
            Trend::FortyFiveUp => "↗",
            Trend::Flat => "→",
            Trend::FortyFiveDown => "↘",
            Trend::SingleDown => "↓",
            Trend::DoubleDown => "⇊",
            Trend::Unknown => "",
        }
    }

    /// Dexcom / Nightscout trend names.
    pub fn from_name(s: &str) -> Self {
        match s {
            "DoubleUp" => Trend::DoubleUp,
            "SingleUp" => Trend::SingleUp,
            "FortyFiveUp" => Trend::FortyFiveUp,
            "Flat" => Trend::Flat,
            "FortyFiveDown" => Trend::FortyFiveDown,
            "SingleDown" => Trend::SingleDown,
            "DoubleDown" => Trend::DoubleDown,
            _ => Trend::Unknown,
        }
    }

    /// Older Dexcom Share responses use numbers 0..9.
    pub fn from_number(n: i64) -> Self {
        match n {
            1 => Trend::DoubleUp,
            2 => Trend::SingleUp,
            3 => Trend::FortyFiveUp,
            4 => Trend::Flat,
            5 => Trend::FortyFiveDown,
            6 => Trend::SingleDown,
            7 => Trend::DoubleDown,
            _ => Trend::Unknown,
        }
    }

    /// From a rate of change in mg/dL per minute (Dexcom's thresholds).
    pub fn from_rate(rate: f64) -> Self {
        if rate > 3.0 {
            Trend::DoubleUp
        } else if rate > 2.0 {
            Trend::SingleUp
        } else if rate > 1.0 {
            Trend::FortyFiveUp
        } else if rate >= -1.0 {
            Trend::Flat
        } else if rate >= -2.0 {
            Trend::FortyFiveDown
        } else if rate >= -3.0 {
            Trend::SingleDown
        } else {
            Trend::DoubleDown
        }
    }

    /// +2 double up, +1 up, 0 flat/unknown, -1 down, -2 double down.
    pub fn direction(self) -> i8 {
        match self {
            Trend::DoubleUp => 2,
            Trend::SingleUp | Trend::FortyFiveUp => 1,
            Trend::Flat | Trend::Unknown => 0,
            Trend::FortyFiveDown | Trend::SingleDown => -1,
            Trend::DoubleDown => -2,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Reading {
    pub mgdl: u16,
    pub trend: Trend,
    /// Unix time in milliseconds.
    pub time_ms: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Units {
    #[default]
    Mgdl,
    Mmol,
}

impl Units {
    pub fn label(self) -> &'static str {
        match self {
            Units::Mgdl => "mg/dL",
            Units::Mmol => "mmol/L",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Thresholds {
    pub urgent_low: u16,
    pub low: u16,
    pub high: u16,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            urgent_low: 55,
            low: 70,
            high: 180,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    InRange,
    High,
    Low,
    UrgentLow,
    Stale,
    NoData,
    SignIn,
    Error,
    Loading,
}

impl Status {
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Status::InRange => [0x2E, 0x9E, 0x5B],
            Status::High => [0xE0, 0x9A, 0x12],
            Status::Low => [0xD6, 0x45, 0x45],
            Status::UrgentLow => [0x8E, 0x1B, 0x1B],
            Status::Stale => [0x8A, 0x92, 0x9C],
            Status::NoData | Status::SignIn | Status::Error | Status::Loading => [0x5D, 0x64, 0x6D],
        }
    }

    /// True when the number should be shown at all.
    pub fn shows_value(self) -> bool {
        matches!(
            self,
            Status::InRange | Status::High | Status::Low | Status::UrgentLow | Status::Stale
        )
    }
}

pub fn age_minutes(r: &Reading, now_ms: i64) -> i64 {
    ((now_ms - r.time_ms).max(0)) / 60_000
}

/// Range / staleness of the latest reading. Never shows an old number as current:
/// 10+ minutes old is Stale (grey), 20+ minutes is NoData (number hidden).
pub fn classify(latest: Option<&Reading>, now_ms: i64, t: &Thresholds) -> Status {
    let Some(r) = latest else {
        return Status::NoData;
    };
    let age = age_minutes(r, now_ms);
    if age >= NO_DATA_MIN {
        Status::NoData
    } else if age >= STALE_MIN {
        Status::Stale
    } else if r.mgdl < t.urgent_low {
        Status::UrgentLow
    } else if r.mgdl < t.low {
        Status::Low
    } else if r.mgdl > t.high {
        Status::High
    } else {
        Status::InRange
    }
}

/// Full value for text (popup, tooltip, badge). Dexcom reports <40 as LOW and >400 as HIGH.
pub fn format_value(mgdl: u16, units: Units) -> String {
    if mgdl < 40 {
        return "LOW".into();
    }
    if mgdl > 400 {
        return "HIGH".into();
    }
    match units {
        Units::Mgdl => mgdl.to_string(),
        Units::Mmol => format!("{:.1}", mgdl as f64 / MMOL_FACTOR),
    }
}

/// Short value for the tiny tray icon: at most 3 characters.
pub fn icon_value(mgdl: u16, units: Units) -> String {
    if mgdl < 40 {
        return "LO".into();
    }
    if mgdl > 400 {
        return "HI".into();
    }
    match units {
        Units::Mgdl => mgdl.to_string(),
        Units::Mmol => {
            let v = mgdl as f64 / MMOL_FACTOR;
            if v >= 9.95 {
                format!("{:.0}", v)
            } else {
                format!("{:.1}", v)
            }
        }
    }
}

pub fn format_delta(delta_mgdl: i32, units: Units) -> String {
    match units {
        Units::Mgdl => format!("{:+}", delta_mgdl),
        Units::Mmol => format!("{:+.1}", delta_mgdl as f64 / MMOL_FACTOR),
    }
}

pub fn to_units(mgdl: u16, units: Units) -> f64 {
    match units {
        Units::Mgdl => mgdl as f64,
        Units::Mmol => (mgdl as f64 / MMOL_FACTOR * 10.0).round() / 10.0,
    }
}

/// Adds newly fetched readings to the ones already held: sorted, one per timestamp
/// (newest data wins), and nothing older than 3 hours 10 minutes.
pub fn merge_readings(old: Vec<Reading>, new: Vec<Reading>, now: i64) -> Vec<Reading> {
    let cutoff = now - (3 * 60 + 10) * 60_000;
    let mut all: Vec<Reading> = old
        .into_iter()
        .chain(new)
        .filter(|r| r.time_ms >= cutoff)
        .collect();
    all.sort_by_key(|r| r.time_ms);
    // Keep the last of each equal timestamp, which is the newly fetched one.
    all.reverse();
    all.dedup_by_key(|r| r.time_ms);
    all.reverse();
    all
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(mgdl: u16, mins_ago: i64, now: i64) -> Reading {
        Reading {
            mgdl,
            trend: Trend::Flat,
            time_ms: now - mins_ago * 60_000,
        }
    }

    #[test]
    fn classify_ranges() {
        let t = Thresholds::default();
        let now = 1_000_000_000_000;
        assert_eq!(classify(Some(&r(120, 2, now)), now, &t), Status::InRange);
        assert_eq!(classify(Some(&r(180, 2, now)), now, &t), Status::InRange);
        assert_eq!(classify(Some(&r(181, 2, now)), now, &t), Status::High);
        assert_eq!(classify(Some(&r(69, 2, now)), now, &t), Status::Low);
        assert_eq!(classify(Some(&r(54, 2, now)), now, &t), Status::UrgentLow);
        assert_eq!(classify(None, now, &t), Status::NoData);
    }

    #[test]
    fn old_readings_are_never_current() {
        let t = Thresholds::default();
        let now = 1_000_000_000_000;
        assert_eq!(classify(Some(&r(50, 9, now)), now, &t), Status::UrgentLow);
        assert_eq!(classify(Some(&r(50, 10, now)), now, &t), Status::Stale);
        assert_eq!(classify(Some(&r(50, 19, now)), now, &t), Status::Stale);
        assert_eq!(classify(Some(&r(50, 20, now)), now, &t), Status::NoData);
        assert!(!Status::NoData.shows_value());
    }

    #[test]
    fn formatting() {
        assert_eq!(format_value(142, Units::Mgdl), "142");
        assert_eq!(format_value(142, Units::Mmol), "7.9");
        assert_eq!(format_value(39, Units::Mgdl), "LOW");
        assert_eq!(format_value(401, Units::Mmol), "HIGH");
        assert_eq!(icon_value(142, Units::Mmol), "7.9");
        assert_eq!(icon_value(188, Units::Mmol), "10");
        assert!(icon_value(400, Units::Mmol).len() <= 3);
        assert_eq!(format_delta(2, Units::Mgdl), "+2");
        assert_eq!(format_delta(-6, Units::Mgdl), "-6");
    }

    #[test]
    fn merge_keeps_three_hours_without_duplicates() {
        let now = 1_700_000_000_000;
        let r = |mgdl, mins_ago: i64| Reading {
            mgdl,
            trend: Trend::Flat,
            time_ms: now - mins_ago * 60_000,
        };
        let old = vec![r(100, 200), r(110, 10), r(120, 5)];
        let new = vec![r(121, 5), r(130, 0)];
        let m = merge_readings(old, new, now);
        assert_eq!(
            m.iter().map(|x| x.mgdl).collect::<Vec<_>>(),
            vec![110, 121, 130]
        );
    }

    #[test]
    fn trends() {
        assert_eq!(Trend::from_name("FortyFiveDown"), Trend::FortyFiveDown);
        assert_eq!(Trend::from_name("NotComputable"), Trend::Unknown);
        assert_eq!(Trend::from_number(4), Trend::Flat);
        assert_eq!(Trend::from_rate(0.5), Trend::Flat);
        assert_eq!(Trend::from_rate(-3.5), Trend::DoubleDown);
    }
}
