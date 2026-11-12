//! Month simulator for the alarm engine (tests only).
//!
//! Drives the real `alarms::Engine` minute by minute over weeks of synthetic glucose with
//! meals, overnight lows, exercise dips and sensor gaps, plus a simulated user who
//! acknowledges, snoozes or sleeps through alerts. Nothing here is real patient data.
//!
//! Run a scenario file and print a JSON report:
//! `CGM_SIM=scenario.json cargo test sim::month -- --ignored --nocapture`
//! Optional `CGM_SIM_EVENTS=events.csv` writes every alert and user action.

use crate::alarms::{About, AlarmSettings, Clock, Engine, Kind};
use crate::config::{Person, Scenario, SourceConfig};
use crate::model::{Reading, Thresholds, Trend, Units};
use crate::sound::Sound;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;

const MIN: i64 = 60_000;
const DAY: i64 = 1440;
/// A Monday 00:00 (the clock's weekday/minute are derived from it, not from the OS).
const BASE_MS: i64 = 1_700_438_400_000;

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct SimConfig {
    pub days: u32,
    pub people: Vec<SimPerson>,
    pub settings: AlarmSettings,
    pub behavior: Behavior,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            days: 30,
            people: vec![SimPerson::default()],
            settings: AlarmSettings::default(),
            behavior: Behavior::default(),
        }
    }
}

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct SimPerson {
    pub name: String,
    pub units: Units,
    pub seed: u64,
    /// 1.0 = typical swings; higher means bigger spikes and more lows.
    pub variability: f64,
    /// Chance per night of an overnight low.
    pub night_low_chance: f64,
    pub alarms_on: bool,
}

impl Default for SimPerson {
    fn default() -> Self {
        SimPerson {
            name: "Sim".into(),
            units: Units::Mgdl,
            seed: 1,
            variability: 1.0,
            night_low_chance: 0.15,
            alarms_on: true,
        }
    }
}

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct Behavior {
    /// Minutes the user takes to react to a notification or sound while awake. None: never reacts.
    pub react_min: Option<i64>,
    /// Asleep between these local minutes (overnight allowed); only sound wakes them.
    pub sleep: (u16, u16),
    /// React with snooze instead of acknowledge.
    pub snooze: bool,
    /// Manual "Quiet for N minutes" presses: (day, minute of day, minutes).
    pub manual_quiet: Vec<(u32, u16, u32)>,
}

impl Default for Behavior {
    fn default() -> Self {
        Behavior {
            react_min: Some(3),
            sleep: (23 * 60, 7 * 60),
            snooze: false,
            manual_quiet: vec![],
        }
    }
}

/// Small deterministic PRNG (xorshift64*), so runs are reproducible.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn f(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.f()
    }
}

/// A bump that peaks `peak_min` after `start` and fades over about 3 × peak.
fn bump(t: f64, start: f64, peak_min: f64, height: f64) -> f64 {
    let x = (t - start) / peak_min;
    if x <= 0.0 {
        0.0
    } else {
        height * x * (1.0 - x).exp()
    }
}

/// One glucose value per 5 minutes for the whole run, plus sensor gaps (None).
fn glucose(p: &SimPerson, days: u32) -> Vec<Option<u16>> {
    let mut rng = Rng::new(p.seed);
    let mut events: Vec<(f64, f64, f64)> = vec![]; // (start minute, peak after, height)
    let mut gaps: Vec<(i64, i64)> = vec![];
    let v = p.variability;
    for d in 0..days as i64 {
        let day = (d * DAY) as f64;
        for meal in [7.5, 12.5, 18.5] {
            let start = day + meal * 60.0 + rng.range(-45.0, 45.0);
            events.push((start, rng.range(45.0, 75.0), rng.range(40.0, 130.0) * v));
            // Insulin a little too strong sometimes: a dip 2.5-4 h after the meal.
            if rng.f() < 0.25 * v {
                events.push((
                    start + rng.range(150.0, 210.0),
                    40.0,
                    -rng.range(35.0, 70.0) * v,
                ));
            }
        }
        if rng.f() < p.night_low_chance {
            events.push((day + rng.range(60.0, 240.0), 50.0, -rng.range(60.0, 95.0)));
        }
        if rng.f() < 0.2 {
            events.push((
                day + rng.range(15.0, 19.0) * 60.0,
                35.0,
                -rng.range(30.0, 60.0) * v,
            ));
        }
        if rng.f() < 0.12 {
            let start = d * DAY + rng.range(0.0, 1380.0) as i64;
            gaps.push((start, start + rng.range(20.0, 90.0) as i64));
        }
    }
    let steps = days as i64 * DAY / 5;
    let mut out = Vec::with_capacity(steps as usize);
    let mut drift = 0.0;
    for k in 0..steps {
        let t = (k * 5) as f64;
        drift = (drift + rng.range(-1.5, 1.5)).clamp(-25.0, 25.0);
        let mut g = 125.0 + drift + rng.range(-3.0, 3.0);
        for &(s, pk, h) in &events {
            if t > s && t < s + pk * 6.0 {
                g += bump(t, s, pk, h);
            }
        }
        let minute = k * 5;
        let in_gap = gaps.iter().any(|&(a, b)| minute >= a && minute < b);
        out.push((!in_gap).then(|| g.round().clamp(39.0, 401.0) as u16));
    }
    out
}

#[derive(Serialize, Default, Debug)]
pub struct PersonReport {
    pub name: String,
    pub readings: usize,
    pub gap_minutes: i64,
    pub pct_below_70: f64,
    pub pct_below_55: f64,
    pub pct_above_180: f64,
    pub pct_above_250: f64,
    /// "low 70 fired" -> count, "low 70 repeat" -> count, ...
    pub alerts: BTreeMap<String, u32>,
    pub notifications: u32,
    pub sounds: u32,
    /// Alerts whose notification or sound a quiet mode removed.
    pub notify_suppressed_by_quiet: u32,
    pub sound_suppressed_by_quiet: u32,
    pub acks: u32,
    pub snoozes: u32,
    /// Minutes from first alert to the user's reaction.
    pub max_unanswered_min: i64,
    /// Fresh readings under 55 for 15+ minutes in a row with no sound (or notification
    /// while awake) in that time.
    pub silent_urgent_lows: u32,
    /// Minutes under 55 (fresh) with no sound or awake notification in the past 30 minutes.
    pub urgent_minutes_unheard_30m: u32,
    /// Sounds not played because a more urgent tone played the same minute.
    pub sounds_dropped: u32,
    pub no_data_alerts: u32,
    /// Fresh readings under 70 for 15+ minutes with no alarm active at all.
    pub missed_lows: u32,
    pub minutes_flashing: i64,
}

#[derive(Serialize, Default, Debug)]
pub struct Report {
    pub days: u32,
    pub people: Vec<PersonReport>,
}

fn asleep(b: &Behavior, minute: u16) -> bool {
    let (a, z) = b.sleep;
    if a == z {
        false
    } else if a < z {
        (a..z).contains(&minute)
    } else {
        minute >= a || minute < z
    }
}

pub fn run(cfg: &SimConfig, mut events: Option<&mut String>) -> Report {
    let mut settings = cfg.settings.clone();
    settings.normalize();
    let people: Vec<Person> = cfg
        .people
        .iter()
        .enumerate()
        .map(|(i, p)| Person {
            id: format!("p{i}"),
            name: p.name.clone(),
            initials: "S".into(),
            color: "#000000".into(),
            units: p.units,
            thresholds: Thresholds::default(),
            source: SourceConfig::Demo {
                scenario: Scenario::Normal,
            },
            tray_icon: true,
        })
        .collect();
    for (i, p) in cfg.people.iter().enumerate() {
        if !p.alarms_on {
            settings.off_for.push(format!("p{i}"));
        }
    }
    let series: Vec<Vec<Option<u16>>> = cfg.people.iter().map(|p| glucose(p, cfg.days)).collect();
    let mut engine = Engine::default();
    let mut reports: Vec<PersonReport> = cfg
        .people
        .iter()
        .map(|p| PersonReport {
            name: p.name.clone(),
            ..Default::default()
        })
        .collect();
    // Per person: last fresh reading, first unanswered alert time, run lengths.
    let mut latest: Vec<Option<Reading>> = vec![None; people.len()];
    let mut pending: Vec<Option<(i64, bool)>> = vec![None; people.len()]; // (since, heard)
    let mut silent_run = vec![0i64; people.len()];
    let mut missed_run = vec![0i64; people.len()];
    // Minute each person's alarms were last heard (sound, or a notification while awake).
    let mut last_heard = vec![i64::MIN / 2; people.len()];
    let total = cfg.days as i64 * DAY;
    for m in 0..total {
        let c = Clock {
            now: BASE_MS + m * MIN,
            weekday: ((m / DAY) % 7) as u8,
            minute: (m % DAY) as u16,
        };
        for &(day, minute, mins) in &cfg.behavior.manual_quiet {
            if day as i64 * DAY + minute as i64 == m {
                engine.set_quiet(mins, c.now);
                log(&mut events, c, "-", "manual quiet", &format!("{mins} min"));
            }
        }
        // A new reading arrives every 5 minutes (the app polls about then).
        if m % 5 == 0 {
            for (i, s) in series.iter().enumerate() {
                let k = (m / 5) as usize;
                match s[k] {
                    Some(v) => {
                        let prev = (1..=3).find_map(|j| k.checked_sub(j).and_then(|q| s[q]));
                        let trend = prev.map_or(Trend::Flat, |p| {
                            Trend::from_rate((v as f64 - p as f64) / 5.0)
                        });
                        latest[i] = Some(Reading {
                            mgdl: v,
                            trend,
                            time_ms: c.now,
                        });
                        let r = &mut reports[i];
                        r.readings += 1;
                        let pct = |b: bool| if b { 1.0 } else { 0.0 };
                        r.pct_below_70 += pct(v < 70);
                        r.pct_below_55 += pct(v < 55);
                        r.pct_above_180 += pct(v > 180);
                        r.pct_above_250 += pct(v > 250);
                    }
                    None => reports[i].gap_minutes += 5,
                }
            }
        }
        let alerts = engine.tick(
            &settings,
            &people,
            |id| latest[id[1..].parse::<usize>().unwrap()].as_ref(),
            c,
        );
        // The app plays only the most urgent tone of a check (like `alarms::deliver`).
        let loudest = alerts
            .iter()
            .map(|a| a.actions.sound)
            .max()
            .unwrap_or(Sound::Off);
        let mut played = false;
        for a in &alerts {
            let i: usize = a.person_id[1..].parse().unwrap();
            let r = &mut reports[i];
            let label = match a.about {
                About::Alarm(Kind::Low) => "low",
                About::Alarm(Kind::High) => "high",
                About::NoData => "no data",
            };
            if a.about == About::NoData {
                r.no_data_alerts += 1;
            } else {
                let key = format!(
                    "{label} {} {}",
                    a.mgdl,
                    if a.repeat { "repeat" } else { "fired" }
                );
                *r.alerts.entry(key).or_default() += 1;
            }
            if a.wanted.notify && !a.actions.notify {
                r.notify_suppressed_by_quiet += 1;
            }
            if a.wanted.sound != Sound::Off && a.actions.sound == Sound::Off {
                r.sound_suppressed_by_quiet += 1;
            }
            r.notifications += a.actions.notify as u32;
            let loud = a.actions.sound != Sound::Off;
            if loud && a.actions.sound == loudest && !played {
                played = true;
                r.sounds += 1;
            } else if loud {
                r.sounds_dropped += 1;
            }
            let heard = loud || (a.actions.notify && !asleep(&cfg.behavior, c.minute));
            if heard {
                last_heard[i] = m;
            }
            if a.about != About::NoData {
                let p = pending[i].get_or_insert((m, false));
                p.1 |= heard;
            }
            log(
                &mut events,
                c,
                &a.title,
                &format!(
                    "{label} {}{}",
                    a.mgdl,
                    if a.repeat { " (repeat)" } else { "" }
                ),
                &format!(
                    "{} notify={} sound={:?} flash={}",
                    a.body, a.actions.notify, a.actions.sound, a.actions.flash
                ),
            );
        }
        // The simulated user reacts some minutes after hearing an alert.
        for (i, p) in people.iter().enumerate() {
            let Some((since, heard)) = pending[i] else {
                continue;
            };
            let still = engine.active.keys().any(|k| k.0 == p.id);
            if !still {
                pending[i] = None;
                continue;
            }
            let reports_i = &mut reports[i];
            let waited = m - since;
            reports_i.max_unanswered_min = reports_i.max_unanswered_min.max(waited);
            let awake = !asleep(&cfg.behavior, c.minute);
            let noticed = heard || awake;
            if let Some(react) = cfg.behavior.react_min {
                if noticed && waited >= react && (awake || heard) {
                    if cfg.behavior.snooze {
                        engine.snooze(&settings, Some(&p.id), c.now);
                        reports_i.snoozes += 1;
                        log(&mut events, c, &p.name, "user", "snooze");
                    } else {
                        engine.acknowledge(&settings, Some(&p.id), c.now);
                        reports_i.acks += 1;
                        log(&mut events, c, &p.name, "user", "acknowledge");
                    }
                    pending[i] = None;
                }
            }
        }
        if engine.any_flashing(&settings, c) {
            for (i, p) in people.iter().enumerate() {
                if engine.view(&settings, p, c).is_some_and(|v| v.flash) {
                    reports[i].minutes_flashing += 1;
                }
            }
        }
        // Coverage checks on fresh readings.
        for (i, p) in people.iter().enumerate() {
            let fresh = latest[i].as_ref().filter(|r| c.now - r.time_ms < 10 * MIN);
            let v = fresh.map(|r| r.mgdl);
            let low_active = engine.active.contains_key(&(p.id.clone(), Kind::Low));
            if v.is_some_and(|v| v < 55) && cfg.people[i].alarms_on && m - last_heard[i] > 30 {
                reports[i].urgent_minutes_unheard_30m += 1;
            }
            if v.is_some_and(|v| v < 55) {
                silent_run[i] += 1;
                let heard_in_run = m - last_heard[i] < silent_run[i];
                if silent_run[i] == 15 && !heard_in_run && cfg.people[i].alarms_on {
                    reports[i].silent_urgent_lows += 1;
                    log(
                        &mut events,
                        c,
                        &p.name,
                        "CHECK",
                        "urgent low 15 min without sound or notification",
                    );
                }
            } else {
                silent_run[i] = 0;
            }
            if v.is_some_and(|v| v < 70) && !low_active {
                missed_run[i] += 1;
                if missed_run[i] == 15 && cfg.people[i].alarms_on {
                    reports[i].missed_lows += 1;
                    log(
                        &mut events,
                        c,
                        &p.name,
                        "CHECK",
                        "below 70 for 15 min with no alarm",
                    );
                }
            } else {
                missed_run[i] = 0;
            }
        }
    }
    for r in &mut reports {
        let n = r.readings.max(1) as f64;
        for x in [
            &mut r.pct_below_70,
            &mut r.pct_below_55,
            &mut r.pct_above_180,
            &mut r.pct_above_250,
        ] {
            *x = (*x / n * 1000.0).round() / 10.0;
        }
    }
    Report {
        days: cfg.days,
        people: reports,
    }
}

fn log(out: &mut Option<&mut String>, c: Clock, who: &str, what: &str, detail: &str) {
    if let Some(out) = out {
        const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let day = (c.now - BASE_MS) / (DAY * MIN) + 1;
        let _ = writeln!(
            out,
            "day {day:>2} {} {:02}:{:02},{who},{what},{detail}",
            DAYS[c.weekday as usize],
            c.minute / 60,
            c.minute % 60
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quiet::{Allowed, QuietMode};

    fn cfg(days: u32) -> SimConfig {
        SimConfig {
            days,
            people: vec![
                SimPerson {
                    name: "A".into(),
                    seed: 7,
                    ..Default::default()
                },
                SimPerson {
                    name: "B".into(),
                    seed: 11,
                    variability: 1.4,
                    night_low_chance: 0.4,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn week_with_defaults_alerts_every_low() {
        let r = run(&cfg(7), None);
        for p in &r.people {
            assert!(p.readings > 1500, "{p:?}");
            assert_eq!(p.missed_lows, 0, "{p:?}");
            assert_eq!(p.silent_urgent_lows, 0, "{p:?}");
            assert!(p.alerts.keys().any(|k| k.starts_with("low")), "{p:?}");
        }
    }

    #[test]
    fn quiet_mode_removes_sound_and_is_counted() {
        let mut c = cfg(7);
        c.settings.quiet = vec![QuietMode {
            name: "Always".into(),
            days: (0..7).collect(),
            start: 0,
            end: 0,
            allow: Allowed {
                flash: true,
                notify: false,
                sound: false,
            },
            ..Default::default()
        }];
        let r = run(&c, None);
        for p in &r.people {
            assert_eq!(p.sounds, 0);
            assert_eq!(p.notifications, 0);
        }
        assert!(r.people.iter().any(|p| p.sound_suppressed_by_quiet > 0));
    }

    /// The audit's parent case: night lows, a parent who wakes only for sound and goes
    /// back to sleep after acknowledging. Every urgent low must keep sounding.
    #[test]
    fn sleeping_parent_hears_every_urgent_low() {
        for snooze in [false, true] {
            let mut c = cfg(21);
            c.people[0].night_low_chance = 0.8;
            c.behavior = Behavior {
                react_min: Some(2),
                sleep: (22 * 60 + 30, 6 * 60 + 30),
                snooze,
                manual_quiet: vec![],
            };
            let r = run(&c, None);
            for p in &r.people {
                assert!(
                    p.pct_below_55 > 0.0,
                    "the scenario must include urgent lows"
                );
                assert_eq!(p.silent_urgent_lows, 0, "snooze={snooze} {p:?}");
                assert!(p.urgent_minutes_unheard_30m <= 15, "snooze={snooze} {p:?}");
            }
        }
    }

    #[test]
    fn simulation_is_deterministic() {
        let a = serde_json::to_string(&run(&cfg(3), None)).unwrap();
        let b = serde_json::to_string(&run(&cfg(3), None)).unwrap();
        assert_eq!(a, b);
    }

    /// `CGM_SIM=scenario.json cargo test sim::tests::month -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn month() {
        let path = std::env::var("CGM_SIM").expect("set CGM_SIM to a scenario JSON file");
        let cfg: SimConfig =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).expect("scenario");
        let mut events = String::from("time,who,what,detail\n");
        let report = run(&cfg, Some(&mut events));
        if let Ok(out) = std::env::var("CGM_SIM_EVENTS") {
            std::fs::write(out, &events).unwrap();
        }
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    }
}
