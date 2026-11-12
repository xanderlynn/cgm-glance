//! Synthetic readings for testing without any account or network.
//! Every scenario is generated from a formula; nothing here is real patient data.

use super::FetchError;
use crate::config::{Person, Scenario, SourceConfig};
use crate::model::{now_ms, Reading, Thresholds, Trend, Units};
use std::f64::consts::TAU;

/// In fast mode, one simulated 5-minute reading arrives every 10 real seconds.
pub const FAST_STEP_SECS: u64 = 10;
const STEP_MS: i64 = 5 * 60_000;
const HISTORY: i64 = 36; // 3 hours

pub struct DemoSource {
    pub scenario: Scenario,
    pub fast: bool,
    seed: f64,
    start_ms: i64,
}

impl DemoSource {
    pub fn new(scenario: Scenario, fast: bool, seed: u64) -> Self {
        DemoSource {
            scenario,
            fast,
            seed: (seed % 997) as f64,
            start_ms: now_ms(),
        }
    }

    pub fn fetch(&mut self) -> Result<Vec<Reading>, FetchError> {
        self.readings_at(now_ms())
    }

    pub fn readings_at(&self, now: i64) -> Result<Vec<Reading>, FetchError> {
        match self.scenario {
            Scenario::SignIn => {
                return Err(FetchError::Auth(
                    "Demo: simulated wrong password. Use \"Sign in again\" to test that flow."
                        .into(),
                ))
            }
            Scenario::Offline => {
                return Err(FetchError::Network(
                    "Demo: simulated network outage.".into(),
                ))
            }
            _ => {}
        }
        // Simulated clock (minutes) and the timestamp of the newest reading.
        let (sim_now, newest_ms) = if self.fast {
            let steps = (now - self.start_ms).max(0) / 1000 / FAST_STEP_SECS as i64;
            (self.seed * 37.0 + steps as f64 * 5.0, now - 60_000)
        } else {
            let newest = now - now.rem_euclid(STEP_MS);
            (newest as f64 / 60_000.0, newest)
        };
        let lag_ms = match self.scenario {
            Scenario::Stale => 14 * 60_000,
            Scenario::NoData => 35 * 60_000,
            _ => 0,
        };
        let mut out: Vec<Reading> = (0..HISTORY)
            .rev()
            .map(|k| Reading {
                mgdl: self.value(sim_now - 5.0 * k as f64),
                trend: Trend::Unknown,
                time_ms: newest_ms - lag_ms - k * STEP_MS,
            })
            .collect();
        for i in 1..out.len() {
            let rate = (out[i].mgdl as f64 - out[i - 1].mgdl as f64) / 5.0;
            out[i].trend = Trend::from_rate(rate);
        }
        out[0].trend = Trend::Flat;
        Ok(out)
    }

    fn value(&self, m: f64) -> u16 {
        let s = self.seed;
        let wave = |period: f64, phase: f64| ((m + phase) / period * TAU).sin();
        let v = match self.scenario {
            Scenario::Normal | Scenario::Stale | Scenario::NoData => {
                130.0 + 72.0 * wave(360.0, s * 13.0) + 14.0 * wave(47.0, s)
            }
            Scenario::Steady => 112.0 + 5.0 * wave(60.0, s),
            Scenario::High => 238.0 + 28.0 * wave(120.0, s),
            Scenario::Low => 64.0 + 4.0 * wave(40.0, s),
            Scenario::UrgentLow => 49.0 + 3.0 * wave(30.0, s),
            Scenario::SignIn | Scenario::Offline => 120.0,
        };
        v.round().clamp(39.0, 401.0) as u16
    }
}

fn demo_person(name: &str, initials: &str, color: &str, scenario: Scenario) -> Person {
    Person {
        id: format!("demo-{}", uuid::Uuid::new_v4()),
        name: name.into(),
        initials: initials.into(),
        color: color.into(),
        units: Units::Mgdl,
        thresholds: Thresholds::default(),
        source: SourceConfig::Demo { scenario },
        tray_icon: true,
    }
}

/// People added automatically when the app starts with `--demo` and has no one yet.
pub fn seed_people() -> Vec<Person> {
    vec![
        demo_person("Alex (demo)", "AL", "#4C6FFF", Scenario::Normal),
        demo_person("Mia (demo)", "MJ", "#E0559B", Scenario::Low),
        demo_person("Sam (demo)", "SK", "#14A3A3", Scenario::Stale),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{classify, Status};

    fn latest_status(s: Scenario) -> Status {
        let now = 1_700_000_000_000;
        let d = DemoSource::new(s, false, 3);
        let r = d.readings_at(now).unwrap();
        classify(r.last(), now, &Thresholds::default())
    }

    #[test]
    fn scenarios_hit_their_states() {
        assert_eq!(latest_status(Scenario::High), Status::High);
        assert_eq!(latest_status(Scenario::Low), Status::Low);
        assert_eq!(latest_status(Scenario::UrgentLow), Status::UrgentLow);
        assert_eq!(latest_status(Scenario::Steady), Status::InRange);
        assert_eq!(latest_status(Scenario::Stale), Status::Stale);
        assert_eq!(latest_status(Scenario::NoData), Status::NoData);
    }

    #[test]
    fn error_scenarios() {
        let d = DemoSource::new(Scenario::SignIn, false, 1);
        assert!(matches!(d.readings_at(0), Err(FetchError::Auth(_))));
        let d = DemoSource::new(Scenario::Offline, false, 1);
        assert!(matches!(d.readings_at(0), Err(FetchError::Network(_))));
    }

    #[test]
    fn three_hours_sorted() {
        let d = DemoSource::new(Scenario::Normal, true, 5);
        let r = d.readings_at(now_ms()).unwrap();
        assert_eq!(r.len(), 36);
        assert!(r.windows(2).all(|w| w[0].time_ms < w[1].time_ms));
    }
}
