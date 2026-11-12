//! Quiet modes: schedules (e.g. Work, Sleep) that limit what alarms may do.

use serde::{Deserialize, Serialize};

pub const MAX_MODES: usize = 8;
const DAY_MIN: u16 = 24 * 60;

/// Which alarm actions are allowed while a mode is on.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Allowed {
    pub flash: bool,
    pub notify: bool,
    pub sound: bool,
}

impl Default for Allowed {
    fn default() -> Self {
        Allowed {
            flash: true,
            notify: true,
            sound: true,
        }
    }
}

impl Allowed {
    fn and(self, o: Allowed) -> Allowed {
        Allowed {
            flash: self.flash && o.flash,
            notify: self.notify && o.notify,
            sound: self.sound && o.sound,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct QuietMode {
    pub name: String,
    pub enabled: bool,
    /// 0 = Monday … 6 = Sunday. The days the window starts on.
    pub days: Vec<u8>,
    /// Minutes after midnight. `end` before `start` runs overnight; equal means all day.
    pub start: u16,
    pub end: u16,
    pub allow: Allowed,
}

impl Default for QuietMode {
    fn default() -> Self {
        QuietMode {
            name: "Quiet".into(),
            enabled: true,
            days: (0..7).collect(),
            start: 22 * 60,
            end: 7 * 60,
            allow: Allowed {
                flash: true,
                notify: false,
                sound: false,
            },
        }
    }
}

/// Examples to edit, off until the user turns them on.
pub fn default_modes() -> Vec<QuietMode> {
    vec![
        QuietMode {
            name: "Work".into(),
            enabled: false,
            days: (0..5).collect(),
            start: 9 * 60,
            end: 17 * 60,
            allow: Allowed {
                flash: true,
                notify: true,
                sound: false,
            },
        },
        QuietMode {
            name: "Focus".into(),
            enabled: false,
            days: (0..5).collect(),
            start: 9 * 60,
            end: 11 * 60,
            allow: Allowed {
                flash: true,
                notify: false,
                sound: false,
            },
        },
    ]
}

impl QuietMode {
    /// True when the mode covers `minute` (0..1440) on `weekday` (0 = Monday).
    pub fn covers(&self, weekday: u8, minute: u16) -> bool {
        if !self.enabled {
            return false;
        }
        let on = |d: u8| self.days.contains(&d);
        let yesterday = (weekday + 6) % 7;
        if self.start == self.end {
            on(weekday)
        } else if self.start < self.end {
            on(weekday) && (self.start..self.end).contains(&minute)
        } else {
            // Overnight: the part after midnight belongs to the day it started on.
            (on(weekday) && minute >= self.start) || (on(yesterday) && minute < self.end)
        }
    }

    pub fn normalize(&mut self) {
        self.name = self.name.trim().chars().take(30).collect();
        if self.name.is_empty() {
            self.name = "Quiet".into();
        }
        self.days.retain(|&d| d < 7);
        self.days.sort_unstable();
        self.days.dedup();
        self.start = self.start.min(DAY_MIN - 1);
        self.end = self.end.min(DAY_MIN - 1);
    }
}

/// The modes on right now and what they still allow (all of them combined).
pub fn active(modes: &[QuietMode], weekday: u8, minute: u16) -> Option<(Vec<String>, Allowed)> {
    let on: Vec<&QuietMode> = modes.iter().filter(|m| m.covers(weekday, minute)).collect();
    if on.is_empty() {
        return None;
    }
    let allow = on.iter().fold(Allowed::default(), |a, m| a.and(m.allow));
    Some((on.iter().map(|m| m.name.clone()).collect(), allow))
}

/// "Quiet for 1 hour": flashing continues, notifications and sound stop.
pub const MANUAL: Allowed = Allowed {
    flash: true,
    notify: false,
    sound: false,
};

/// Local weekday (0 = Monday) and minute of the day.
pub fn local_now() -> (u8, u16) {
    use chrono::{Datelike, Timelike};
    let t = chrono::Local::now();
    (
        t.weekday().num_days_from_monday() as u8,
        (t.hour() * 60 + t.minute()) as u16,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(days: &[u8], start: u16, end: u16) -> QuietMode {
        QuietMode {
            days: days.to_vec(),
            start,
            end,
            ..Default::default()
        }
    }

    const MON: u8 = 0;
    const FRI: u8 = 4;
    const SAT: u8 = 5;
    const SUN: u8 = 6;

    #[test]
    fn daytime_window() {
        let work = mode(&[0, 1, 2, 3, 4], 9 * 60, 17 * 60);
        assert!(work.covers(MON, 9 * 60));
        assert!(work.covers(FRI, 16 * 60 + 59));
        assert!(!work.covers(FRI, 17 * 60), "end is exclusive");
        assert!(!work.covers(MON, 8 * 60 + 59));
        assert!(!work.covers(SAT, 12 * 60));
    }

    #[test]
    fn overnight_window_belongs_to_its_start_day() {
        // Sleep Friday and Saturday nights only, 22:00 to 07:00.
        let sleep = mode(&[FRI, SAT], 22 * 60, 7 * 60);
        assert!(sleep.covers(FRI, 23 * 60));
        assert!(
            sleep.covers(SAT, 6 * 60),
            "Friday night continues into Saturday"
        );
        assert!(
            sleep.covers(SUN, 3 * 60),
            "Saturday night continues into Sunday"
        );
        assert!(!sleep.covers(SUN, 23 * 60), "Sunday night isn't selected");
        assert!(!sleep.covers(FRI, 6 * 60), "Thursday night isn't selected");
        assert!(!sleep.covers(SAT, 7 * 60));
    }

    #[test]
    fn week_wraps_from_sunday_to_monday() {
        let sun_night = mode(&[SUN], 22 * 60, 6 * 60);
        assert!(sun_night.covers(MON, 60));
    }

    #[test]
    fn equal_times_mean_all_day() {
        let m = mode(&[SAT], 0, 0);
        assert!(m.covers(SAT, 0) && m.covers(SAT, 1439));
        assert!(!m.covers(SUN, 0));
    }

    #[test]
    fn disabled_modes_never_cover() {
        let mut m = mode(&[MON], 0, 0);
        m.enabled = false;
        assert!(!m.covers(MON, 600));
    }

    #[test]
    fn overlapping_modes_combine_restrictively() {
        let mut a = mode(&[MON], 9 * 60, 17 * 60);
        a.name = "Work".into();
        a.allow = Allowed {
            flash: true,
            notify: true,
            sound: false,
        };
        let mut b = mode(&[MON], 10 * 60, 11 * 60);
        b.name = "Focus".into();
        b.allow = Allowed {
            flash: true,
            notify: false,
            sound: true,
        };
        let (names, allow) = active(&[a.clone(), b.clone()], MON, 10 * 60 + 30).unwrap();
        assert_eq!(names, vec!["Work", "Focus"]);
        assert_eq!(
            allow,
            Allowed {
                flash: true,
                notify: false,
                sound: false
            }
        );
        assert!(active(&[a, b], SUN, 600).is_none());
    }

    #[test]
    fn normalize_cleans_input() {
        let mut m = mode(&[9, 3, 3, 1], 5000, 20);
        m.name = "   ".into();
        m.normalize();
        assert_eq!(m.days, vec![1, 3]);
        assert_eq!(m.start, 1439);
        assert_eq!(m.name, "Quiet");
    }
}
