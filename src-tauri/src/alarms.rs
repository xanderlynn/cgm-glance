//! Escalating glucose alarms.
//!
//! A phase is a threshold (e.g. 70 mg/dL), an optional trend condition and what to do.
//! The most severe phase the latest reading is in wins. An alarm fires when the reading
//! enters a more severe phase than the one already active, and clears once the reading
//! recovers a little past the least severe phase. Old readings never trigger anything.

use crate::config::Person;
use crate::model::{self, Reading, Units};
use crate::quiet::{self, Allowed, QuietMode};
use crate::sound::{self, Sound};
use crate::state::{AppState, Inner};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
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
    pub sound: Sound,
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
                sound: Sound::Off,
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

    fn sound(mut self, s: Sound) -> Self {
        self.actions.sound = s;
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
    /// How long "Snooze" silences alarms.
    pub snooze_min: u32,
    /// Schedules that limit what alarms may do (they can silence any alarm).
    pub quiet: Vec<QuietMode>,
}

impl Default for AlarmSettings {
    fn default() -> Self {
        AlarmSettings {
            enabled: true,
            low: vec![
                phase(75, true),
                phase(70, false).notify(),
                phase(65, false).notify().sound(Sound::Urgent),
            ],
            high: vec![phase(200, true), phase(250, false).notify()],
            off_for: vec![],
            repeat_min: 5,
            snooze_min: 30,
            quiet: quiet::default_modes(),
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
        self.snooze_min = self.snooze_min.clamp(5, 240);
        self.quiet.truncate(quiet::MAX_MODES);
        self.quiet.iter_mut().for_each(QuietMode::normalize);
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

/// The quiet modes on right now (scheduled or manual) and what alarms may still do.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct QuietView {
    /// e.g. "Work" or "Quiet for now".
    pub name: String,
    pub allow: Allowed,
    /// Unix ms when a manual quiet ends.
    pub manual_until: Option<i64>,
}

/// Something to tell the user now: an alarm fired or repeated. `actions` already
/// leave out whatever the current quiet modes don't allow.
#[derive(Debug, Clone)]
// `person_id`, `kind` and `mgdl` are read by the month simulator.
#[cfg_attr(not(test), allow(dead_code))]
pub struct Alert {
    pub person_id: String,
    pub kind: Kind,
    pub mgdl: u16,
    pub title: String,
    pub body: String,
    pub actions: Actions,
}

/// A point in time, in both forms the alarms need.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub now: i64,
    /// 0 = Monday.
    pub weekday: u8,
    /// Minutes after local midnight.
    pub minute: u16,
}

impl Clock {
    pub fn local() -> Clock {
        let (weekday, minute) = quiet::local_now();
        Clock {
            now: model::now_ms(),
            weekday,
            minute,
        }
    }
}

/// All alarm state. Pure logic over an explicit clock, so the month simulator can
/// drive exactly what the app runs.
#[derive(Default, Debug)]
pub struct Engine {
    /// Alarms that have fired and not cleared, per person and kind.
    pub active: HashMap<(String, Kind), Active>,
    /// Alarms stay quiet until this time (ms), unless they escalate.
    pub snoozed_until: Option<i64>,
    /// Manual "Quiet for 1 hour" end time (ms).
    pub quiet_until: Option<i64>,
}

impl Engine {
    pub fn snoozed(&self, now: i64) -> Option<i64> {
        self.snoozed_until.filter(|&t| now < t)
    }

    pub fn quiet(&self, s: &AlarmSettings, c: Clock) -> Option<QuietView> {
        let scheduled = quiet::active(&s.quiet, c.weekday, c.minute);
        let manual = self.quiet_until.filter(|&t| c.now < t);
        let mut names: Vec<String> = scheduled.as_ref().map(|q| q.0.clone()).unwrap_or_default();
        let mut allow = scheduled.map(|q| q.1).unwrap_or_default();
        if manual.is_some() {
            names.push("Quiet for now".into());
            allow = allow.and(quiet::MANUAL);
        }
        (!names.is_empty()).then(|| QuietView {
            name: names.join(" + "),
            allow,
            manual_until: manual,
        })
    }

    fn allowed(&self, s: &AlarmSettings, c: Clock) -> Allowed {
        self.quiet(s, c).map(|q| q.allow).unwrap_or_default()
    }

    /// Flashing is on for an unacknowledged alarm whose phase flashes, unless snoozed
    /// or a quiet mode doesn't allow it.
    fn flashes(&self, s: &AlarmSettings, kind: Kind, a: &Active, c: Clock) -> bool {
        !a.acked
            && self.snoozed(c.now).is_none()
            && self.allowed(s, c).flash
            && phase_of(s.phases(kind), a).is_some_and(|p| p.actions.flash)
    }

    pub fn view(&self, s: &AlarmSettings, p: &Person, c: Clock) -> Option<AlarmView> {
        Kind::ALL.into_iter().find_map(|kind| {
            let a = self.active.get(&(p.id.clone(), kind))?;
            Some(AlarmView {
                kind,
                threshold: threshold_text(a.mgdl, p.units),
                flash: self.flashes(s, kind, a, c),
                acked: a.acked,
            })
        })
    }

    pub fn any_flashing(&self, s: &AlarmSettings, c: Clock) -> bool {
        self.active
            .iter()
            .any(|((_, kind), a)| self.flashes(s, *kind, a, c))
    }

    /// Runs every alarm against each person's latest reading and returns what to
    /// alert now.
    pub fn tick<'a>(
        &mut self,
        s: &AlarmSettings,
        people: &[Person],
        latest: impl Fn(&str) -> Option<&'a Reading>,
        c: Clock,
    ) -> Vec<Alert> {
        let now = c.now;
        let allow = self.allowed(s, c);
        let mut alerts = vec![];
        self.active
            .retain(|(id, _), _| people.iter().any(|p| &p.id == id));
        for p in people {
            let latest = latest(&p.id);
            for kind in Kind::ALL {
                let key = (p.id.clone(), kind);
                let mut active = self.active.remove(&key);
                if s.on_for(&p.id) {
                    let phases = s.phases(kind);
                    let fired = step(kind, phases, &mut active, latest, now).cloned();
                    // A more severe phase always alerts and ends a snooze.
                    if fired.is_some() {
                        self.snoozed_until = None;
                    }
                    let snoozed = self.snoozed(now).is_some();
                    let repeat = active.as_mut().and_then(|a| {
                        if fired.is_none() && !snoozed && repeat_due(a, s.repeat_min, now) {
                            a.alerted_ms = now;
                            phase_of(phases, a).cloned()
                        } else {
                            None
                        }
                    });
                    if let (Some(ph), Some(r)) = (fired.or(repeat), latest) {
                        let mut alert = alert_for(p, kind, &ph, r);
                        alert.actions.notify &= allow.notify;
                        if !allow.sound {
                            alert.actions.sound = Sound::Off;
                        }
                        alert.actions.flash &= allow.flash;
                        alerts.push(alert);
                    }
                } else {
                    active = None;
                }
                if let Some(a) = active {
                    self.active.insert(key, a);
                }
            }
        }
        alerts
    }

    /// Acknowledges one person's alarms, or everyone's when `id` is None.
    pub fn acknowledge(&mut self, id: Option<&str>) {
        for ((pid, _), a) in self.active.iter_mut() {
            if id.map_or(true, |id| id == pid) {
                a.acked = true;
            }
        }
    }

    pub fn snooze(&mut self, s: &AlarmSettings, now: i64) {
        self.snoozed_until = Some(now + s.snooze_min as i64 * 60_000);
    }

    /// Starts a manual quiet period, or ends it when `minutes` is 0.
    pub fn set_quiet(&mut self, minutes: u32, now: i64) {
        self.quiet_until = (minutes > 0).then(|| now + minutes.min(24 * 60) as i64 * 60_000);
    }
}

fn threshold_text(mgdl: u16, units: Units) -> String {
    match units {
        Units::Mgdl => mgdl.to_string(),
        Units::Mmol => format!("{:.1}", model::to_units(mgdl, units)),
    }
}

fn alert_for(p: &Person, kind: Kind, phase: &Phase, r: &Reading) -> Alert {
    let (word, edge) = match kind {
        Kind::Low => ("low", "at or below"),
        Kind::High => ("high", "at or above"),
    };
    let units = p.units;
    Alert {
        person_id: p.id.clone(),
        kind,
        mgdl: phase.mgdl,
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

// ---------- App glue ----------

/// Runs every alarm against the latest readings. Called before each refresh.
pub fn check(app: &AppHandle) {
    let state = app.state::<AppState>();
    let c = Clock::local();
    let (alerts, flashing) = {
        let mut guard = state.lock();
        let inner = &mut *guard;
        let settings = &inner.config.settings.alarms;
        let runtime = &inner.runtime;
        let alerts = inner.engine.tick(
            settings,
            &inner.config.people,
            |id| runtime.get(id).and_then(|r| r.readings.last()),
            c,
        );
        (alerts, inner.engine.any_flashing(settings, c))
    };
    if flashing {
        crate::ui::ensure_flasher(app);
    }
    for a in alerts {
        deliver(app, &a);
    }
}

pub fn any_flashing(inner: &Inner) -> bool {
    inner
        .engine
        .any_flashing(&inner.config.settings.alarms, Clock::local())
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
    sound::play(a.actions.sound);
}

/// Applies a change to the alarm engine (acknowledge, snooze, quiet) with the settings.
pub fn with_engine(app: &AppHandle, f: impl FnOnce(&mut Engine, &AlarmSettings, i64)) {
    let state = app.state::<AppState>();
    let mut guard = state.lock();
    let inner = &mut *guard;
    f(
        &mut inner.engine,
        &inner.config.settings.alarms,
        model::now_ms(),
    );
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
        assert_eq!(low()[2].actions.sound, Sound::Urgent);
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
