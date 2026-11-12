//! Escalating glucose alarms.
//!
//! A phase is a threshold (e.g. 70 mg/dL), an optional trend condition and what to do.
//! The most severe phase the latest reading is in wins. An alarm fires when the reading
//! enters a more severe phase than the one already active, and clears once the reading
//! recovers a little past the least severe phase. Old readings never trigger anything.

use crate::config::Person;
use crate::model::{self, Reading, Units};
use crate::state::{AppState, Inner};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use tauri::{AppHandle, Manager};

pub const MAX_PHASES: usize = 6;
/// How far past the least severe phase a reading must recover to clear the alarm.
pub const CLEAR_MARGIN: u16 = 5;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Low,
    High,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Low, Kind::High];

    /// True when threshold `a` is more severe than `b`.
    fn more_severe(self, a: u16, b: u16) -> bool {
        match self {
            Kind::Low => a < b,
            Kind::High => a > b,
        }
    }

    fn in_phase(self, p: &Phase, r: &Reading) -> bool {
        let dir = r.trend.direction();
        match self {
            Kind::Low => r.mgdl <= p.mgdl && (!p.trend_only || dir < 0),
            Kind::High => r.mgdl >= p.mgdl && (!p.trend_only || dir > 0),
        }
    }

    fn recovered(self, mgdl: u16, least_severe: u16) -> bool {
        match self {
            Kind::Low => mgdl > least_severe.saturating_add(CLEAR_MARGIN),
            Kind::High => mgdl < least_severe.saturating_sub(CLEAR_MARGIN),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Actions {
    pub flash: bool,
    pub notify: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Phase {
    pub enabled: bool,
    /// Threshold in mg/dL.
    pub mgdl: u16,
    /// Only while the trend points toward the threshold (falling for lows, rising for highs).
    pub trend_only: bool,
    pub actions: Actions,
}

impl Default for Phase {
    fn default() -> Self {
        Phase {
            enabled: true,
            mgdl: 70,
            trend_only: false,
            actions: Actions {
                flash: true,
                notify: false,
            },
        }
    }
}

fn phase(mgdl: u16, trend_only: bool) -> Phase {
    Phase {
        mgdl,
        trend_only,
        ..Default::default()
    }
}

impl Phase {
    fn notify(mut self) -> Self {
        self.actions.notify = true;
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AlarmSettings {
    pub enabled: bool,
    pub low: Vec<Phase>,
    pub high: Vec<Phase>,
    /// People whose alarms are turned off. The rules are shared by everyone else.
    pub off_for: Vec<String>,
    /// An unacknowledged alarm alerts again (notification, sound) after this many minutes.
    pub repeat_min: u32,
}

impl Default for AlarmSettings {
    fn default() -> Self {
        AlarmSettings {
            enabled: true,
            low: vec![
                phase(75, true),
                phase(70, false).notify(),
                phase(65, false).notify(),
            ],
            high: vec![phase(200, true), phase(250, false).notify()],
            off_for: vec![],
            repeat_min: 5,
        }
    }
}

impl AlarmSettings {
    pub fn phases(&self, kind: Kind) -> &[Phase] {
        match kind {
            Kind::Low => &self.low,
            Kind::High => &self.high,
        }
    }

    /// Keeps saved rules sane: at most `MAX_PHASES`, 40 to 400 mg/dL, least severe first.
    pub fn normalize(&mut self) {
        self.repeat_min = self.repeat_min.clamp(1, 60);
        for kind in Kind::ALL {
            let phases = match kind {
                Kind::Low => &mut self.low,
                Kind::High => &mut self.high,
            };
            phases.truncate(MAX_PHASES);
            for p in phases.iter_mut() {
                p.mgdl = p.mgdl.clamp(40, 400);
            }
            phases.sort_by(|a, b| {
                if kind.more_severe(a.mgdl, b.mgdl) {
                    Ordering::Greater
                } else if kind.more_severe(b.mgdl, a.mgdl) {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            });
        }
    }

    pub fn on_for(&self, id: &str) -> bool {
        self.enabled && !self.off_for.iter().any(|x| x == id)
    }
}

/// An alarm that has fired and not yet cleared.
#[derive(Clone, Debug, PartialEq)]
pub struct Active {
    /// Threshold of the phase that fired; identifies the phase even if the list is edited.
    pub mgdl: u16,
    /// When it last alerted (fired or repeated).
    pub alerted_ms: i64,
    pub acked: bool,
}

/// True when an unacknowledged alarm should alert again.
pub fn repeat_due(a: &Active, repeat_min: u32, now: i64) -> bool {
    !a.acked && now - a.alerted_ms >= repeat_min as i64 * 60_000
}

/// The most severe enabled phase the reading is in. Old readings match nothing.
pub fn matching<'a>(kind: Kind, phases: &'a [Phase], r: &Reading, now: i64) -> Option<&'a Phase> {
    if model::age_minutes(r, now) >= model::STALE_MIN {
        return None;
    }
    phases
        .iter()
        .filter(|p| p.enabled && kind.in_phase(p, r))
        .reduce(|a, b| {
            if kind.more_severe(b.mgdl, a.mgdl) {
                b
            } else {
                a
            }
        })
}

/// Advances one alarm for the latest reading. Returns the phase that just fired, if any.
pub fn step<'a>(
    kind: Kind,
    phases: &'a [Phase],
    active: &mut Option<Active>,
    latest: Option<&Reading>,
    now: i64,
) -> Option<&'a Phase> {
    let least = phases
        .iter()
        .filter(|p| p.enabled)
        .map(|p| p.mgdl)
        .reduce(|a, b| if kind.more_severe(a, b) { b } else { a });
    let Some(least) = least else {
        *active = None;
        return None;
    };
    let r = latest?;
    if let Some(p) = matching(kind, phases, r, now) {
        if active
            .as_ref()
            .map_or(true, |a| kind.more_severe(p.mgdl, a.mgdl))
        {
            *active = Some(Active {
                mgdl: p.mgdl,
                alerted_ms: now,
                acked: false,
            });
            return Some(p);
        }
    } else if model::age_minutes(r, now) < model::STALE_MIN && kind.recovered(r.mgdl, least) {
        *active = None;
    }
    None
}

/// The phase an active alarm belongs to, if it still exists.
pub fn phase_of<'a>(phases: &'a [Phase], a: &Active) -> Option<&'a Phase> {
    phases.iter().find(|p| p.enabled && p.mgdl == a.mgdl)
}

/// What the windows show about a person's alarm.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AlarmView {
    pub kind: Kind,
    /// The phase threshold in the person's units, e.g. "70" or "3.9".
    pub threshold: String,
    pub flash: bool,
    pub acked: bool,
}

pub fn view(inner: &Inner, p: &Person) -> Option<AlarmView> {
    let settings = &inner.config.settings.alarms;
    Kind::ALL.into_iter().find_map(|kind| {
        let a = inner.alarms.get(&(p.id.clone(), kind))?;
        let phase = phase_of(settings.phases(kind), a);
        Some(AlarmView {
            kind,
            threshold: threshold_text(a.mgdl, p.units),
            flash: !a.acked && phase.is_some_and(|ph| ph.actions.flash),
            acked: a.acked,
        })
    })
}

fn threshold_text(mgdl: u16, units: Units) -> String {
    match units {
        Units::Mgdl => mgdl.to_string(),
        Units::Mmol => format!("{:.1}", model::to_units(mgdl, units)),
    }
}

pub fn any_flashing(inner: &Inner) -> bool {
    let settings = &inner.config.settings.alarms;
    inner.alarms.iter().any(|((_, kind), a)| {
        !a.acked && phase_of(settings.phases(*kind), a).is_some_and(|p| p.actions.flash)
    })
}

/// Something to tell the user now: an alarm fired or repeated.
#[derive(Debug, Clone)]
pub struct Alert {
    pub title: String,
    pub body: String,
    pub actions: Actions,
}

fn alert_for(p: &Person, kind: Kind, phase: &Phase, r: &Reading) -> Alert {
    let (word, edge) = match kind {
        Kind::Low => ("low", "at or below"),
        Kind::High => ("high", "at or above"),
    };
    let units = p.units;
    Alert {
        title: format!("{} is {word}", p.name),
        body: format!(
            "{} {} {} · alarm {edge} {}",
            model::format_value(r.mgdl, units),
            units.label(),
            r.trend.arrow(),
            threshold_text(phase.mgdl, units)
        ),
        actions: phase.actions.clone(),
    }
}

/// Runs every alarm against the latest readings. Called before each refresh.
pub fn check(app: &AppHandle) {
    let state = app.state::<AppState>();
    let now = model::now_ms();
    let mut alerts = vec![];
    let flashing = {
        let mut guard = state.lock();
        let inner = &mut *guard;
        let settings = &inner.config.settings.alarms;
        let people = &inner.config.people;
        inner
            .alarms
            .retain(|(id, _), _| people.iter().any(|p| &p.id == id));
        for p in people {
            let latest = inner.runtime.get(&p.id).and_then(|r| r.readings.last());
            for kind in Kind::ALL {
                let key = (p.id.clone(), kind);
                let mut active = inner.alarms.remove(&key);
                if settings.on_for(&p.id) {
                    let phases = settings.phases(kind);
                    let fired = step(kind, phases, &mut active, latest, now).cloned();
                    let repeat = active.as_mut().and_then(|a| {
                        if fired.is_none() && repeat_due(a, settings.repeat_min, now) {
                            a.alerted_ms = now;
                            phase_of(phases, a).cloned()
                        } else {
                            None
                        }
                    });
                    if let (Some(ph), Some(r)) = (fired.or(repeat), latest) {
                        alerts.push(alert_for(p, kind, &ph, r));
                    }
                } else {
                    active = None;
                }
                if let Some(a) = active {
                    inner.alarms.insert(key, a);
                }
            }
        }
        any_flashing(inner)
    };
    if flashing {
        crate::ui::ensure_flasher(app);
    }
    for a in alerts {
        deliver(app, &a);
    }
}

fn deliver(app: &AppHandle, a: &Alert) {
    if a.actions.notify {
        use tauri_plugin_notification::NotificationExt;
        if let Err(e) = app
            .notification()
            .builder()
            .title(&a.title)
            .body(&a.body)
            .show()
        {
            eprintln!("cgm-glance: notification failed: {e}");
        }
    }
}

/// Acknowledges the alarms of one person, or of everyone when `id` is None.
pub fn acknowledge(app: &AppHandle, id: Option<&str>) {
    let state = app.state::<AppState>();
    let mut inner = state.lock();
    for ((pid, _), a) in inner.alarms.iter_mut() {
        if id.map_or(true, |id| id == pid) {
            a.acked = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Trend;

    const NOW: i64 = 1_700_000_000_000;

    fn r(mgdl: u16, trend: Trend, age_min: i64) -> Reading {
        Reading {
            mgdl,
            trend,
            time_ms: NOW - age_min * 60_000,
        }
    }

    fn low() -> Vec<Phase> {
        AlarmSettings::default().low
    }

    #[test]
    fn trend_condition_and_severity() {
        let p = low();
        // 74 flat: the 75 phase needs a falling trend, so nothing matches.
        assert!(matching(Kind::Low, &p, &r(74, Trend::Flat, 1), NOW).is_none());
        assert_eq!(
            matching(Kind::Low, &p, &r(74, Trend::FortyFiveDown, 1), NOW)
                .unwrap()
                .mgdl,
            75
        );
        assert_eq!(
            matching(Kind::Low, &p, &r(70, Trend::Flat, 1), NOW)
                .unwrap()
                .mgdl,
            70
        );
        assert_eq!(
            matching(Kind::Low, &p, &r(60, Trend::SingleDown, 1), NOW)
                .unwrap()
                .mgdl,
            65
        );
    }

    #[test]
    fn old_readings_never_match() {
        assert!(matching(Kind::Low, &low(), &r(50, Trend::SingleDown, 10), NOW).is_none());
    }

    #[test]
    fn disabled_phases_are_skipped() {
        let mut p = low();
        p[2].enabled = false;
        assert_eq!(
            matching(Kind::Low, &p, &r(60, Trend::Flat, 1), NOW)
                .unwrap()
                .mgdl,
            70
        );
    }

    #[test]
    fn escalates_only_to_more_severe_phases() {
        let p = low();
        let mut a = None;
        assert_eq!(
            step(
                Kind::Low,
                &p,
                &mut a,
                Some(&r(74, Trend::SingleDown, 0)),
                NOW
            )
            .unwrap()
            .mgdl,
            75
        );
        // Same phase again: no new firing.
        assert!(step(
            Kind::Low,
            &p,
            &mut a,
            Some(&r(73, Trend::SingleDown, 0)),
            NOW
        )
        .is_none());
        assert_eq!(
            step(
                Kind::Low,
                &p,
                &mut a,
                Some(&r(69, Trend::SingleDown, 0)),
                NOW
            )
            .unwrap()
            .mgdl,
            70
        );
        assert_eq!(
            step(Kind::Low, &p, &mut a, Some(&r(64, Trend::Flat, 0)), NOW)
                .unwrap()
                .mgdl,
            65
        );
        // Rising back through a milder phase keeps the more severe alarm.
        assert!(step(Kind::Low, &p, &mut a, Some(&r(69, Trend::SingleUp, 0)), NOW).is_none());
        assert_eq!(a.as_ref().unwrap().mgdl, 65);
    }

    #[test]
    fn clears_only_after_recovering_past_the_margin() {
        let p = low();
        let mut a = None;
        step(Kind::Low, &p, &mut a, Some(&r(68, Trend::Flat, 0)), NOW);
        // 78 is above 75 but within the margin: still active.
        step(Kind::Low, &p, &mut a, Some(&r(78, Trend::SingleUp, 0)), NOW);
        assert!(a.is_some());
        step(Kind::Low, &p, &mut a, Some(&r(81, Trend::SingleUp, 0)), NOW);
        assert!(a.is_none());
    }

    #[test]
    fn old_reading_neither_fires_nor_clears() {
        let p = low();
        let mut a = None;
        step(Kind::Low, &p, &mut a, Some(&r(68, Trend::Flat, 0)), NOW);
        step(Kind::Low, &p, &mut a, Some(&r(120, Trend::Flat, 12)), NOW);
        assert!(a.is_some());
    }

    #[test]
    fn no_enabled_phases_clears() {
        let mut p = low();
        let mut a = None;
        step(Kind::Low, &p, &mut a, Some(&r(60, Trend::Flat, 0)), NOW);
        p.iter_mut().for_each(|x| x.enabled = false);
        step(Kind::Low, &p, &mut a, Some(&r(60, Trend::Flat, 0)), NOW);
        assert!(a.is_none());
    }

    #[test]
    fn normalize_sorts_and_clamps() {
        let mut s = AlarmSettings {
            low: vec![phase(20, false), phase(80, true), phase(70, false)],
            ..Default::default()
        };
        s.low.extend((0..10).map(|i| phase(100 + i, false)));
        s.normalize();
        assert!(s.low.len() <= MAX_PHASES);
        let order: Vec<u16> = s.low.iter().map(|p| p.mgdl).collect();
        assert!(order.windows(2).all(|w| w[0] >= w[1]), "{order:?}");
        assert!(order.iter().all(|&m| (40..=400).contains(&m)));
    }

    fn high() -> Vec<Phase> {
        AlarmSettings::default().high
    }

    #[test]
    fn high_phases_escalate_and_clear() {
        let p = high();
        let mut a = None;
        // 210 flat: the 200 phase needs a rising trend.
        assert!(step(Kind::High, &p, &mut a, Some(&r(210, Trend::Flat, 0)), NOW).is_none());
        assert_eq!(
            step(
                Kind::High,
                &p,
                &mut a,
                Some(&r(210, Trend::FortyFiveUp, 0)),
                NOW
            )
            .unwrap()
            .mgdl,
            200
        );
        assert_eq!(
            step(Kind::High, &p, &mut a, Some(&r(260, Trend::Flat, 0)), NOW)
                .unwrap()
                .mgdl,
            250
        );
        step(
            Kind::High,
            &p,
            &mut a,
            Some(&r(198, Trend::SingleDown, 0)),
            NOW,
        );
        assert!(a.is_some(), "within the clearing margin");
        step(
            Kind::High,
            &p,
            &mut a,
            Some(&r(194, Trend::SingleDown, 0)),
            NOW,
        );
        assert!(a.is_none());
    }

    #[test]
    fn high_normalize_sorts_least_severe_first() {
        let mut s = AlarmSettings {
            high: vec![phase(300, false), phase(200, false), phase(250, false)],
            ..Default::default()
        };
        s.normalize();
        let order: Vec<u16> = s.high.iter().map(|p| p.mgdl).collect();
        assert_eq!(order, vec![200, 250, 300]);
    }

    #[test]
    fn repeats_until_acknowledged() {
        let mut a = Active {
            mgdl: 70,
            alerted_ms: NOW,
            acked: false,
        };
        assert!(!repeat_due(&a, 5, NOW + 4 * 60_000));
        assert!(repeat_due(&a, 5, NOW + 5 * 60_000));
        a.acked = true;
        assert!(!repeat_due(&a, 5, NOW + 60 * 60_000));
    }

    #[test]
    fn alert_text() {
        let p = Person {
            id: "x".into(),
            name: "Mia".into(),
            initials: "MJ".into(),
            color: "#000".into(),
            units: Units::Mgdl,
            thresholds: Default::default(),
            source: crate::config::SourceConfig::Demo {
                scenario: crate::config::Scenario::Low,
            },
            tray_icon: true,
        };
        let a = alert_for(&p, Kind::Low, &low()[1], &r(68, Trend::SingleDown, 0));
        assert_eq!(a.title, "Mia is low");
        assert_eq!(a.body, "68 mg/dL ↓ · alarm at or below 70");
        assert!(a.actions.notify);
    }

    #[test]
    fn off_for_person() {
        let s = AlarmSettings {
            off_for: vec!["a".into()],
            ..Default::default()
        };
        assert!(!s.on_for("a"));
        assert!(s.on_for("b"));
    }
}
