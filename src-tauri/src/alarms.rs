//! Escalating glucose alarms.
//!
//! A phase is a threshold (e.g. 70 mg/dL), an optional trend condition and what to do.
//! On every check the alarm follows the most severe phase the latest reading is in:
//! entering a more severe phase alerts at once; moving to a milder one just changes what
//! later repeats do. Repeats happen only while a phase still matches (or while data is
//! missing, so a low never goes quiet because the sensor dropped out). An alarm clears
//! once the reading recovers a little past the least severe phase. Fresh readings older
//! than 10 minutes never start an alarm.

use crate::config::Person;
use crate::model::{self, Reading, Units};
use crate::quiet::{self, Allowed, QuietMode};
use crate::sound::{self, Sound};
use crate::state::{AppState, Inner};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tauri::{AppHandle, Manager};

pub const MAX_PHASES: usize = 6;
/// How far past the least severe phase a reading must recover to clear the alarm.
pub const CLEAR_MARGIN: u16 = 5;
const MIN: i64 = 60_000;

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

impl Actions {
    /// Both sets of actions combined (used when merging duplicate phases).
    fn union(&self, o: &Actions) -> Actions {
        Actions {
            flash: self.flash || o.flash,
            notify: self.notify || o.notify,
            sound: self.sound.max(o.sound),
        }
    }

    fn masked(&self, allow: Allowed) -> Actions {
        Actions {
            flash: self.flash && allow.flash,
            notify: self.notify && allow.notify,
            sound: if allow.sound { self.sound } else { Sound::Off },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Phase {
    pub enabled: bool,
    /// Threshold in mg/dL. Unique within a list (`normalize` merges duplicates).
    pub mgdl: u16,
    /// Only while the trend points toward the threshold (falling for lows, rising for highs).
    pub trend_only: bool,
    pub actions: Actions,
    /// Alerts even during quiet modes and snooze.
    pub breakthrough: bool,
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
            breakthrough: false,
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

/// The alert for readings that stop arriving.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NoData {
    /// Minutes without a new reading before alerting; 0 turns it off.
    pub after_min: u32,
    pub actions: Actions,
}

impl Default for NoData {
    fn default() -> Self {
        NoData {
            after_min: 30,
            actions: Actions {
                flash: false,
                notify: true,
                sound: Sound::Chime,
            },
        }
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
    /// An unacknowledged low alerts again after this many minutes.
    pub repeat_min: u32,
    /// An unacknowledged high alerts again after this many minutes.
    pub high_repeat_min: u32,
    /// Acknowledging quiets a low only this long; if it's still low, it alerts again.
    pub low_ack_min: u32,
    pub high_ack_min: u32,
    /// How long "Snooze" silences a person's alarms.
    pub snooze_min: u32,
    pub no_data: NoData,
    /// Put glucose numbers in notifications. Off keeps them off the lock screen and
    /// notification history (the alert still names the person and says low/high).
    pub notification_values: bool,
    /// Schedules that limit what alarms may do (they can silence any alarm without
    /// `breakthrough`).
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
            high_repeat_min: 30,
            low_ack_min: 15,
            high_ack_min: 60,
            snooze_min: 30,
            no_data: NoData::default(),
            notification_values: true,
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

    fn repeat_ms(&self, kind: Kind) -> i64 {
        let m = match kind {
            Kind::Low => self.repeat_min,
            Kind::High => self.high_repeat_min,
        };
        m as i64 * MIN
    }

    fn ack_ms(&self, kind: Kind) -> i64 {
        let m = match kind {
            Kind::Low => self.low_ack_min,
            Kind::High => self.high_ack_min,
        };
        m as i64 * MIN
    }

    /// Keeps saved rules sane: 40 to 400 mg/dL, one phase per threshold (duplicates are
    /// merged so no action is lost), least severe first, at most `MAX_PHASES` (the most
    /// severe are kept).
    pub fn normalize(&mut self) {
        self.repeat_min = self.repeat_min.clamp(1, 60);
        self.high_repeat_min = self.high_repeat_min.clamp(1, 240);
        self.low_ack_min = self.low_ack_min.clamp(5, 120);
        self.high_ack_min = self.high_ack_min.clamp(5, 480);
        self.snooze_min = self.snooze_min.clamp(5, 240);
        self.no_data.after_min = match self.no_data.after_min {
            0 => 0,
            m => m.clamp(15, 240),
        };
        self.quiet.truncate(quiet::MAX_MODES);
        self.quiet.iter_mut().for_each(QuietMode::normalize);
        for kind in Kind::ALL {
            let phases = match kind {
                Kind::Low => &mut self.low,
                Kind::High => &mut self.high,
            };
            for p in phases.iter_mut() {
                p.mgdl = p.mgdl.clamp(40, 400);
            }
            // Least severe first.
            phases.sort_by_key(|p| match kind {
                Kind::Low => u16::MAX - p.mgdl,
                Kind::High => p.mgdl,
            });
            let mut merged: Vec<Phase> = Vec::with_capacity(phases.len());
            for p in phases.drain(..) {
                match merged.last_mut() {
                    Some(m) if m.mgdl == p.mgdl => {
                        m.enabled |= p.enabled;
                        // Merged phase matches whenever either did.
                        m.trend_only &= p.trend_only;
                        m.actions = m.actions.union(&p.actions);
                        m.breakthrough |= p.breakthrough;
                    }
                    _ => merged.push(p),
                }
            }
            let extra = merged.len().saturating_sub(MAX_PHASES);
            merged.drain(..extra);
            *phases = merged;
        }
    }

    pub fn on_for(&self, id: &str) -> bool {
        self.enabled && !self.off_for.iter().any(|x| x == id)
    }
}

/// An alarm that has fired and not yet cleared.
#[derive(Clone, Debug, PartialEq)]
pub struct Active {
    /// Threshold of the phase it is in now (or was last in while data is missing).
    pub mgdl: u16,
    /// When it last alerted (fired or repeated).
    pub alerted_ms: i64,
    /// Acknowledged until this time; then it reminds again if still in a phase.
    pub acked_until: Option<i64>,
    pub snoozed_until: Option<i64>,
}

impl Active {
    fn new(mgdl: u16, now: i64) -> Active {
        Active {
            mgdl,
            alerted_ms: now,
            acked_until: None,
            snoozed_until: None,
        }
    }

    pub fn acked(&self, now: i64) -> bool {
        self.acked_until.is_some_and(|t| now < t)
    }

    pub fn snoozed(&self, now: i64) -> bool {
        self.snoozed_until.is_some_and(|t| now < t)
    }
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

/// What a check decided for one alarm.
#[derive(Debug, PartialEq)]
pub enum Step<'a> {
    Quiet,
    /// Entered a more severe phase: alert now, whatever was acknowledged or snoozed.
    Fired(&'a Phase),
    /// Still in a phase (or data is missing) and due to remind.
    Repeat(&'a Phase),
}

/// Advances one alarm for the latest reading.
pub fn step<'a>(
    kind: Kind,
    s: &'a AlarmSettings,
    active: &mut Option<Active>,
    latest: Option<&Reading>,
    now: i64,
) -> Step<'a> {
    let phases = s.phases(kind);
    let least = phases
        .iter()
        .filter(|p| p.enabled)
        .map(|p| p.mgdl)
        .reduce(|a, b| if kind.more_severe(a, b) { b } else { a });
    let (Some(least), Some(r)) = (least, latest) else {
        if least.is_none() {
            *active = None;
        }
        return Step::Quiet;
    };
    let fresh = model::age_minutes(r, now) < model::STALE_MIN;
    let current = matching(kind, phases, r, now);
    match (active.as_mut(), current) {
        (None, Some(p)) => {
            *active = Some(Active::new(p.mgdl, now));
            return Step::Fired(p);
        }
        (Some(a), Some(p)) if kind.more_severe(p.mgdl, a.mgdl) => {
            *active = Some(Active::new(p.mgdl, now));
            return Step::Fired(p);
        }
        // Milder phase (or the phase was edited): follow it quietly.
        (Some(a), Some(p)) => a.mgdl = p.mgdl,
        (Some(_), None) if fresh && kind.recovered(r.mgdl, least) => {
            *active = None;
            return Step::Quiet;
        }
        _ => {}
    }
    let Some(a) = active.as_mut() else {
        return Step::Quiet;
    };
    // Repeat while a phase matches now, or while data is missing (last known phase).
    let phase = match current {
        Some(p) => Some(p),
        None if !fresh => phases.iter().find(|p| p.enabled && p.mgdl == a.mgdl),
        None => None,
    };
    let Some(p) = phase else {
        return Step::Quiet;
    };
    let held = a.acked(now) || (a.snoozed(now) && !p.breakthrough);
    if !held && now - a.alerted_ms >= s.repeat_ms(kind) {
        a.alerted_ms = now;
        return Step::Repeat(p);
    }
    Step::Quiet
}

/// What the windows show about a person's alarm.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AlarmView {
    pub kind: Kind,
    /// The phase threshold in the person's units, e.g. "70" or "3.9".
    pub threshold: String,
    pub flash: bool,
    pub acked: bool,
    pub snoozed_until: Option<i64>,
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

/// What an alert is about.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum About {
    Alarm(Kind),
    NoData,
}

/// Something to tell the user now. `actions` already leave out what quiet modes don't
/// allow; `wanted` is what the phase asked for.
#[derive(Debug, Clone)]
// `person_id`, `about`, `mgdl`, `repeat` and `wanted` are read by the month simulator.
#[cfg_attr(not(test), allow(dead_code))]
pub struct Alert {
    pub person_id: String,
    pub about: About,
    pub mgdl: u16,
    pub repeat: bool,
    pub title: String,
    pub body: String,
    pub actions: Actions,
    pub wanted: Actions,
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

/// All alarm state. Pure logic over an explicit clock, so the month simulator drives
/// exactly what the app runs.
#[derive(Default, Debug)]
pub struct Engine {
    /// Alarms that have fired and not cleared, per person and kind.
    pub active: HashMap<(String, Kind), Active>,
    /// Manual "Quiet for 1 hour" end time (ms).
    pub quiet_until: Option<i64>,
    /// People already told their readings stopped (until a fresh one arrives).
    no_data_told: HashSet<String>,
    /// What quiet modes allowed at the last check, to notice them ending.
    last_allow: Option<Allowed>,
}

impl Engine {
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

    fn flashes(&self, s: &AlarmSettings, kind: Kind, a: &Active, c: Clock) -> bool {
        let Some(p) = s
            .phases(kind)
            .iter()
            .find(|p| p.enabled && p.mgdl == a.mgdl)
        else {
            return false;
        };
        let allowed = p.breakthrough || (!a.snoozed(c.now) && self.allowed(s, c).flash);
        p.actions.flash && !a.acked(c.now) && allowed
    }

    pub fn view(&self, s: &AlarmSettings, p: &Person, c: Clock) -> Option<AlarmView> {
        // Lows first: they matter most.
        Kind::ALL.into_iter().find_map(|kind| {
            let a = self.active.get(&(p.id.clone(), kind))?;
            Some(AlarmView {
                kind,
                threshold: threshold_text(a.mgdl, p.units),
                flash: self.flashes(s, kind, a, c),
                acked: a.acked(c.now),
                snoozed_until: a.snoozed_until.filter(|&t| c.now < t),
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
        // A quiet mode just ended or relaxed: alerts it held back are due now.
        let relaxed = self
            .last_allow
            .is_some_and(|l| (allow.notify && !l.notify) || (allow.sound && !l.sound));
        self.last_allow = Some(allow);
        if relaxed {
            for a in self.active.values_mut() {
                a.alerted_ms = i64::MIN / 2;
            }
        }
        let mut alerts = vec![];
        self.active
            .retain(|(id, _), _| people.iter().any(|p| &p.id == id));
        for p in people {
            let latest = latest(&p.id);
            let on = s.on_for(&p.id);
            if let Some(alert) = self.check_no_data(s, p, latest, on, now) {
                let mut alert = alert;
                alert.actions = alert.wanted.masked(allow);
                alerts.push(alert);
            }
            for kind in Kind::ALL {
                let key = (p.id.clone(), kind);
                let mut active = self.active.remove(&key);
                if on {
                    let (phase, repeat) = match step(kind, s, &mut active, latest, now) {
                        Step::Fired(ph) => (Some(ph), false),
                        Step::Repeat(ph) => (Some(ph), true),
                        Step::Quiet => (None, false),
                    };
                    if let (Some(ph), Some(r)) = (phase, latest) {
                        let mut alert = alert_for(p, kind, ph, r, now);
                        if !s.notification_values {
                            alert.body = "Open CGM Glance to see the reading.".into();
                        }
                        alert.repeat = repeat;
                        if !ph.breakthrough {
                            alert.actions = alert.wanted.masked(allow);
                        }
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

    fn check_no_data(
        &mut self,
        s: &AlarmSettings,
        p: &Person,
        latest: Option<&Reading>,
        on: bool,
        now: i64,
    ) -> Option<Alert> {
        let r = latest?;
        let age = model::age_minutes(r, now);
        let limit = s.no_data.after_min as i64;
        if age < model::STALE_MIN || !on || limit == 0 {
            self.no_data_told.remove(&p.id);
            return None;
        }
        if age < limit || !self.no_data_told.insert(p.id.clone()) {
            return None;
        }
        Some(Alert {
            person_id: p.id.clone(),
            about: About::NoData,
            mgdl: r.mgdl,
            repeat: false,
            title: format!("No readings for {}", p.name),
            body: if s.notification_values {
                format!(
                    "Nothing new for {age} min. Last: {} {}. Check the sensor and phone.",
                    model::format_value(r.mgdl, p.units),
                    p.units.label()
                )
            } else {
                format!("Nothing new for {age} min. Check the sensor and phone.")
            },
            actions: s.no_data.actions.clone(),
            wanted: s.no_data.actions.clone(),
        })
    }

    /// Acknowledges one person's alarms, or everyone's when `id` is None. They remind
    /// again after the acknowledge time if still in a phase.
    pub fn acknowledge(&mut self, s: &AlarmSettings, id: Option<&str>, now: i64) {
        for ((pid, kind), a) in self.active.iter_mut() {
            if id.map_or(true, |id| id == pid) {
                a.acked_until = Some(now + s.ack_ms(*kind));
                a.alerted_ms = now;
            }
        }
    }

    /// Snoozes one person's alarms, or everyone's when `id` is None. A more severe
    /// phase still alerts and ends the snooze, and a low is never snoozed for longer
    /// than its acknowledge reminder, so snoozing at 3 a.m. can't hide a falling child.
    pub fn snooze(&mut self, s: &AlarmSettings, id: Option<&str>, now: i64) {
        for ((pid, kind), a) in self.active.iter_mut() {
            if id.map_or(true, |id| id == pid) {
                let mut ms = s.snooze_min as i64 * MIN;
                if *kind == Kind::Low {
                    ms = ms.min(s.ack_ms(Kind::Low));
                }
                a.snoozed_until = Some(now + ms);
            }
        }
    }

    /// Starts a manual quiet period, or ends it when `minutes` is 0.
    pub fn set_quiet(&mut self, minutes: u32, now: i64) {
        self.quiet_until = (minutes > 0).then(|| now + minutes.min(24 * 60) as i64 * MIN);
    }
}

fn threshold_text(mgdl: u16, units: Units) -> String {
    match units {
        Units::Mgdl => mgdl.to_string(),
        Units::Mmol => format!("{:.1}", model::to_units(mgdl, units)),
    }
}

fn alert_for(p: &Person, kind: Kind, phase: &Phase, r: &Reading, now: i64) -> Alert {
    let (word, edge) = match kind {
        Kind::Low => ("low", "at or below"),
        Kind::High => ("high", "at or above"),
    };
    let units = p.units;
    let age = model::age_minutes(r, now);
    let value = format!("{} {}", model::format_value(r.mgdl, units), units.label());
    let (title, reading) = if age >= model::STALE_MIN {
        (
            format!("{} was {word}, no new reading", p.name),
            format!("Last {value} {age} min ago"),
        )
    } else {
        (
            format!("{} is {word}", p.name),
            format!("{value} {}", r.trend.arrow())
                .trim_end()
                .to_string(),
        )
    };
    Alert {
        person_id: p.id.clone(),
        about: About::Alarm(kind),
        mgdl: phase.mgdl,
        repeat: false,
        title,
        body: format!(
            "{reading} · alarm {edge} {}",
            threshold_text(phase.mgdl, units)
        ),
        actions: phase.actions.clone(),
        wanted: phase.actions.clone(),
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
    deliver(app, &alerts);
}

pub fn any_flashing(inner: &Inner) -> bool {
    inner
        .engine
        .any_flashing(&inner.config.settings.alarms, Clock::local())
}

/// Shows each alert's notification, and plays only the most urgent sound among them so
/// a gentle chime never takes the place of an urgent tone.
fn deliver(app: &AppHandle, alerts: &[Alert]) {
    use tauri_plugin_notification::NotificationExt;
    for a in alerts.iter().filter(|a| a.actions.notify) {
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
    if let Some(s) = alerts.iter().map(|a| a.actions.sound).max() {
        sound::play(s);
    }
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
            time_ms: NOW - age_min * MIN,
        }
    }

    fn at(mgdl: u16, trend: Trend, t_min: i64) -> Reading {
        Reading {
            mgdl,
            trend,
            time_ms: NOW + t_min * MIN,
        }
    }

    fn s() -> AlarmSettings {
        AlarmSettings::default()
    }

    fn fired(st: Step) -> Option<u16> {
        match st {
            Step::Fired(p) => Some(p.mgdl),
            _ => None,
        }
    }

    fn repeated(st: Step) -> Option<u16> {
        match st {
            Step::Repeat(p) => Some(p.mgdl),
            _ => None,
        }
    }

    fn mins(m: i64) -> i64 {
        NOW + m * MIN
    }

    #[test]
    fn trend_condition_and_severity() {
        let p = s().low;
        assert!(matching(Kind::Low, &p, &r(74, Trend::Flat, 1), NOW).is_none());
        let m = |v, t| matching(Kind::Low, &p, &r(v, t, 1), NOW).map(|p| p.mgdl);
        assert_eq!(m(74, Trend::FortyFiveDown), Some(75));
        assert_eq!(m(70, Trend::Flat), Some(70));
        assert_eq!(m(60, Trend::SingleDown), Some(65));
    }

    #[test]
    fn old_readings_never_match() {
        assert!(matching(Kind::Low, &s().low, &r(50, Trend::SingleDown, 10), NOW).is_none());
    }

    #[test]
    fn escalates_at_once_and_follows_milder_phases_quietly() {
        let s = s();
        let mut a = None;
        let mut go = |v, t: Trend, m| step(Kind::Low, &s, &mut a, Some(&at(v, t, m)), mins(m));
        assert_eq!(fired(go(74, Trend::SingleDown, 0)), Some(75));
        assert_eq!(go(73, Trend::SingleDown, 1), Step::Quiet);
        assert_eq!(fired(go(69, Trend::SingleDown, 2)), Some(70));
        assert_eq!(fired(go(64, Trend::Flat, 3)), Some(65));
        // Back up to the 70 phase: no alert, and repeats now use the 70 phase.
        assert_eq!(go(68, Trend::SingleUp, 4), Step::Quiet);
        assert_eq!(repeated(go(68, Trend::Flat, 8)), Some(70));
        // Down again into 65: alerts at once.
        assert_eq!(fired(go(64, Trend::SingleDown, 9)), Some(65));
    }

    #[test]
    fn no_repeats_while_no_phase_matches() {
        let s = s();
        let mut a = None;
        step(Kind::Low, &s, &mut a, Some(&at(64, Trend::Flat, 0)), NOW);
        // 78: above every phase but inside the clearing margin. Active, but silent.
        for m in 1..30 {
            assert_eq!(
                step(
                    Kind::Low,
                    &s,
                    &mut a,
                    Some(&at(78, Trend::Flat, m)),
                    mins(m)
                ),
                Step::Quiet
            );
        }
        assert!(a.is_some());
        step(
            Kind::Low,
            &s,
            &mut a,
            Some(&at(81, Trend::SingleUp, 31)),
            mins(31),
        );
        assert!(a.is_none());
    }

    #[test]
    fn trend_only_phase_stops_repeating_once_flat() {
        let s = s();
        let mut a = None;
        step(
            Kind::High,
            &s,
            &mut a,
            Some(&at(210, Trend::SingleUp, 0)),
            NOW,
        );
        for m in 1..60 {
            assert_eq!(
                step(
                    Kind::High,
                    &s,
                    &mut a,
                    Some(&at(210, Trend::Flat, m)),
                    mins(m)
                ),
                Step::Quiet
            );
        }
    }

    #[test]
    fn acknowledging_an_urgent_low_reminds_again() {
        let s = s();
        let mut e = Engine::default();
        let mut a = None;
        step(Kind::Low, &s, &mut a, Some(&at(60, Trend::Flat, 0)), NOW);
        e.active.insert(("x".into(), Kind::Low), a.unwrap());
        e.acknowledge(&s, None, mins(1));
        let mut a = e.active.remove(&("x".into(), Kind::Low));
        let mut reminders = vec![];
        for m in 2..60 {
            if let Some(p) = repeated(step(
                Kind::Low,
                &s,
                &mut a,
                Some(&at(45, Trend::Flat, m)),
                mins(m),
            )) {
                reminders.push((m, p));
            }
        }
        // Quiet for 15 minutes after acknowledging, then every 5 minutes.
        assert_eq!(reminders.first(), Some(&(16, 65)));
        assert!(reminders.len() >= 8, "{reminders:?}");
    }

    #[test]
    fn repeats_during_a_data_gap_use_the_last_phase() {
        let s = s();
        let mut a = None;
        let last = at(60, Trend::SingleDown, 0);
        step(Kind::Low, &s, &mut a, Some(&last), NOW);
        let mut reps = 0;
        for m in 1..40 {
            if repeated(step(Kind::Low, &s, &mut a, Some(&last), mins(m))) == Some(65) {
                reps += 1;
            }
        }
        assert_eq!(reps, 7, "every 5 minutes while no new data arrives");
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
        let alert = alert_for(&p, Kind::Low, &s.low[2], &last, mins(25));
        assert_eq!(alert.title, "Mia was low, no new reading");
        assert_eq!(
            alert.body,
            "Last 60 mg/dL 25 min ago · alarm at or below 65"
        );
        let fresh = alert_for(&p, Kind::Low, &s.low[1], &at(68, Trend::SingleDown, 0), NOW);
        assert_eq!(fresh.title, "Mia is low");
        assert_eq!(fresh.body, "68 mg/dL ↓ · alarm at or below 70");
    }

    #[test]
    fn editing_the_active_phase_keeps_alerting() {
        let mut s = s();
        let mut a = None;
        step(Kind::Low, &s, &mut a, Some(&at(62, Trend::Flat, 0)), NOW);
        // The 65 phase is turned off while sitting at 62: the 70 phase takes over.
        s.low[2].enabled = false;
        let mut got = vec![];
        for m in 1..20 {
            if let Some(p) = repeated(step(
                Kind::Low,
                &s,
                &mut a,
                Some(&at(62, Trend::Flat, m)),
                mins(m),
            )) {
                got.push(p);
            }
        }
        assert!(!got.is_empty() && got.iter().all(|&p| p == 70), "{got:?}");
    }

    #[test]
    fn snooze_is_per_person_and_ends_on_escalation() {
        let s = s();
        let people: Vec<Person> = ["a", "b"]
            .iter()
            .map(|id| Person {
                id: id.to_string(),
                name: id.to_string(),
                initials: "X".into(),
                color: "#000".into(),
                units: Units::Mgdl,
                thresholds: Default::default(),
                source: crate::config::SourceConfig::Demo {
                    scenario: crate::config::Scenario::Low,
                },
                tray_icon: true,
            })
            .collect();
        let clock = |m: i64| Clock {
            now: mins(m),
            weekday: 5,
            minute: 600,
        };
        let mut e = Engine::default();
        let mut values: HashMap<&str, Reading> = HashMap::new();
        values.insert("a", at(68, Trend::Flat, 0));
        values.insert("b", at(68, Trend::Flat, 0));
        let tick = |e: &mut Engine, v: &HashMap<&str, Reading>, m| {
            e.tick(&s, &people, |id| v.get(id), clock(m))
        };
        assert_eq!(tick(&mut e, &values, 0).len(), 2);
        e.snooze(&s, Some("a"), mins(1));
        let until = e.active[&("a".to_string(), Kind::Low)].snoozed_until;
        assert_eq!(
            until,
            Some(mins(16)),
            "a low snoozes no longer than its 15 min reminder"
        );
        let at10 = tick(&mut e, &values, 10);
        assert_eq!(
            at10.iter()
                .map(|a| a.person_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b"],
            "only b repeats"
        );
        values.insert("a", at(60, Trend::Flat, 11));
        let at11 = tick(&mut e, &values, 11);
        assert!(at11.iter().any(|a| a.person_id == "a" && a.mgdl == 65));
    }

    #[test]
    fn quiet_mode_ending_alerts_at_once_and_breakthrough_ignores_it() {
        let mut s = s();
        s.quiet = vec![QuietMode {
            name: "Focus".into(),
            days: (0..7).collect(),
            start: 9 * 60,
            end: 11 * 60,
            allow: Allowed {
                flash: true,
                notify: false,
                sound: false,
            },
            ..Default::default()
        }];
        s.repeat_min = 30;
        let people = vec![Person {
            id: "a".into(),
            name: "A".into(),
            initials: "A".into(),
            color: "#000".into(),
            units: Units::Mgdl,
            thresholds: Default::default(),
            source: crate::config::SourceConfig::Demo {
                scenario: crate::config::Scenario::Low,
            },
            tray_icon: true,
        }];
        let c = |minute: u16| Clock {
            now: NOW + minute as i64 * MIN,
            weekday: 0,
            minute,
        };
        let mut e = Engine::default();
        let rd = |minute: u16| Reading {
            mgdl: 68,
            trend: Trend::Flat,
            time_ms: NOW + minute as i64 * MIN,
        };
        let (r650, r660) = (rd(650), rd(660));
        let first = e.tick(&s, &people, |_| Some(&r650), c(650));
        assert!(!first[0].actions.notify, "held back during Focus");
        assert!(first[0].wanted.notify);
        let at_end = e.tick(&s, &people, |_| Some(&r660), c(660));
        assert!(
            at_end.len() == 1 && at_end[0].actions.notify,
            "alerts the minute Focus ends"
        );

        // A breakthrough phase isn't held back by the mode.
        s.low[2].breakthrough = true;
        let mut e = Engine::default();
        let low = Reading {
            mgdl: 60,
            trend: Trend::Flat,
            time_ms: NOW + 600 * MIN,
        };
        let got = e.tick(&s, &people, |_| Some(&low), c(600));
        assert_eq!(got[0].actions.sound, Sound::Urgent);
    }

    #[test]
    fn no_data_alerts_once_per_gap() {
        let s = s();
        let people = vec![Person {
            id: "a".into(),
            name: "A".into(),
            initials: "A".into(),
            color: "#000".into(),
            units: Units::Mgdl,
            thresholds: Default::default(),
            source: crate::config::SourceConfig::Demo {
                scenario: crate::config::Scenario::Normal,
            },
            tray_icon: true,
        }];
        let last = at(120, Trend::Flat, 0);
        let mut e = Engine::default();
        let mut told = vec![];
        for m in 0..120 {
            let c = Clock {
                now: mins(m),
                weekday: 0,
                minute: 0,
            };
            for a in e.tick(&s, &people, |_| Some(&last), c) {
                assert_eq!(a.about, About::NoData);
                told.push(m);
            }
        }
        assert_eq!(told, vec![30]);
    }

    #[test]
    fn normalize_merges_duplicates_and_keeps_the_most_severe() {
        let mut st = AlarmSettings {
            low: vec![
                phase(70, true),
                phase(70, false).sound(Sound::Urgent),
                phase(10, false),
                phase(30, false).notify(),
            ],
            high: (0..9).map(|i| phase(200 + i * 20, false)).collect(),
            ..Default::default()
        };
        st.normalize();
        assert_eq!(
            st.low.iter().map(|p| p.mgdl).collect::<Vec<_>>(),
            vec![70, 40]
        );
        assert!(!st.low[0].trend_only && st.low[0].actions.sound == Sound::Urgent);
        assert!(
            st.low[1].actions.notify,
            "merged 10 and 30 keep both actions"
        );
        let highs: Vec<u16> = st.high.iter().map(|p| p.mgdl).collect();
        assert_eq!(highs, vec![260, 280, 300, 320, 340, 360]);
    }

    #[test]
    fn notifications_can_leave_out_numbers() {
        let mut s = s();
        s.notification_values = false;
        let people = vec![Person {
            id: "a".into(),
            name: "Mia".into(),
            initials: "M".into(),
            color: "#000".into(),
            units: Units::Mgdl,
            thresholds: Default::default(),
            source: crate::config::SourceConfig::Demo {
                scenario: crate::config::Scenario::Low,
            },
            tray_icon: true,
        }];
        let low = at(60, Trend::SingleDown, 0);
        let c = Clock {
            now: NOW,
            weekday: 0,
            minute: 600,
        };
        let got = Engine::default().tick(&s, &people, |_| Some(&low), c);
        assert_eq!(got[0].title, "Mia is low");
        assert!(
            !got[0].body.chars().any(|ch| ch.is_ascii_digit()),
            "{}",
            got[0].body
        );
    }

    #[test]
    fn off_for_person() {
        let st = AlarmSettings {
            off_for: vec!["a".into()],
            ..Default::default()
        };
        assert!(!st.on_for("a"));
        assert!(st.on_for("b"));
    }
}
