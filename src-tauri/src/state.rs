//! Shared app state and the view model sent to the windows.

use crate::config::{self, Config, Person, Settings, SourceConfig};
use crate::model::{self, Reading, Status, Trend};
use crate::secrets::Secret;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Default)]
pub struct Runtime {
    pub readings: Vec<Reading>,
    /// Last fetch problem. `SignIn` stops polling; anything else is retried.
    pub error: Option<(Status, String)>,
}

/// A tested source waiting for the user to finish the wizard. Memory only.
pub struct Draft {
    pub source: SourceConfig,
    pub secret: Option<Secret>,
    pub created: Instant,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Geometry {
    pub w: u32,
    pub h: u32,
    pub x: i32,
    pub y: i32,
}

#[derive(Default)]
pub struct Inner {
    pub config: Config,
    pub runtime: HashMap<String, Runtime>,
    pub drafts: HashMap<String, Draft>,
    pub pollers: HashMap<String, tauri::async_runtime::JoinHandle<()>>,
    pub trays: HashSet<String>,
    pub badge_geometry: HashMap<String, Geometry>,
    pub badge_flags: HashMap<String, String>,
    pub move_seq: HashMap<String, u64>,
    pub popup_hidden_at: Option<Instant>,
}

pub struct AppState {
    pub inner: Mutex<Inner>,
    pub path: PathBuf,
    pub demo_mode: bool,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(config: Config, path: PathBuf, demo_mode: bool) -> Self {
        let http = reqwest::Client::builder()
            .https_only(true)
            .timeout(std::time::Duration::from_secs(20))
            .user_agent(concat!("cgm-glance/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client");
        AppState {
            inner: Mutex::new(Inner {
                config,
                ..Default::default()
            }),
            path,
            demo_mode,
            http,
        }
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn save(&self, inner: &Inner) {
        if let Err(e) = config::save(&self.path, &inner.config) {
            eprintln!("cgm-glance: could not save settings: {e}");
        }
    }
}

#[derive(Serialize, Clone)]
pub struct HistPoint {
    pub t: i64,
    pub v: f64,
}

#[derive(Serialize, Clone)]
pub struct PersonView {
    pub id: String,
    pub name: String,
    pub initials: String,
    pub color: String,
    pub source_label: String,
    pub is_demo: bool,
    pub status: Status,
    /// Full value, e.g. "142" or "7.9". None when it must not be shown.
    pub value: Option<String>,
    pub icon_text: Option<String>,
    pub unit_label: String,
    pub arrow: String,
    pub direction: i8,
    pub delta: Option<String>,
    pub age_min: Option<i64>,
    pub message: Option<String>,
    pub history: Vec<HistPoint>,
    pub low: f64,
    pub high: f64,
}

#[derive(Serialize, Clone)]
pub struct Snapshot {
    pub demo_mode: bool,
    pub people: Vec<PersonView>,
    pub settings: Settings,
    pub config_people: Vec<Person>,
}

pub fn person_view(p: &Person, rt: Option<&Runtime>, now: i64) -> PersonView {
    let readings: &[Reading] = rt.map(|r| r.readings.as_slice()).unwrap_or(&[]);
    let latest = readings.last();
    let units = p.units;
    let mut status = model::classify(latest, now, &p.thresholds);
    let mut message = None;
    match rt.and_then(|r| r.error.as_ref()) {
        Some((Status::SignIn, m)) => {
            status = Status::SignIn;
            message = Some(m.clone());
        }
        Some((_, m)) => {
            if latest.is_none() {
                status = Status::Error;
            }
            message = Some(m.clone());
        }
        None if rt.is_none() => status = Status::Loading,
        None => {}
    }
    let show = status.shows_value();
    let value = latest
        .filter(|_| show)
        .map(|r| model::format_value(r.mgdl, units));
    let icon_text = latest
        .filter(|_| show)
        .map(|r| model::icon_value(r.mgdl, units));
    let fresh = matches!(
        status,
        Status::InRange | Status::High | Status::Low | Status::UrgentLow
    );
    let trend = latest
        .filter(|_| fresh)
        .map(|r| r.trend)
        .unwrap_or(Trend::Unknown);
    let delta = match readings {
        [.., a, b] if fresh && b.time_ms - a.time_ms <= 15 * 60_000 => {
            Some(model::format_delta(b.mgdl as i32 - a.mgdl as i32, units))
        }
        _ => None,
    };
    if message.is_none() {
        message = match status {
            Status::NoData if latest.is_some() => {
                Some("No recent reading. Check the phone and Share settings.".into())
            }
            Status::Loading => Some("Getting the first reading…".into()),
            _ => None,
        };
    }
    let cutoff = now - 3 * 3_600_000;
    PersonView {
        id: p.id.clone(),
        name: p.name.clone(),
        initials: p.initials.clone(),
        color: p.color.clone(),
        source_label: p.source.label(),
        is_demo: p.source.is_demo(),
        status,
        value,
        icon_text,
        unit_label: units.label().into(),
        arrow: trend.arrow().into(),
        direction: trend.direction(),
        delta,
        age_min: latest.map(|r| model::age_minutes(r, now)),
        message,
        history: readings
            .iter()
            .filter(|r| r.time_ms >= cutoff)
            .map(|r| HistPoint {
                t: r.time_ms,
                v: model::to_units(r.mgdl, units),
            })
            .collect(),
        low: model::to_units(p.thresholds.low, units),
        high: model::to_units(p.thresholds.high, units),
    }
}

pub fn snapshot(state: &AppState) -> Snapshot {
    let inner = state.lock();
    let now = model::now_ms();
    Snapshot {
        demo_mode: state.demo_mode,
        people: inner
            .config
            .people
            .iter()
            .map(|p| person_view(p, inner.runtime.get(&p.id), now))
            .collect(),
        settings: inner.config.settings.clone(),
        config_people: inner.config.people.clone(),
    }
}

/// One line per person for tooltips: "Alex 142 mg/dL → +2 · 2 min ago".
pub fn summary_line(v: &PersonView) -> String {
    let age = v.age_min.map(|a| {
        if a == 0 {
            "just now".to_string()
        } else {
            format!("{a} min ago")
        }
    });
    let demo = if v.is_demo { " [demo]" } else { "" };
    match (&v.value, v.status) {
        (_, Status::SignIn) => format!("{}{demo}: sign in again", v.name),
        (Some(val), _) => {
            let mut s = format!("{} {} {}", v.name, val, v.unit_label);
            if !v.arrow.is_empty() {
                s += &format!(" {}", v.arrow);
            }
            if let Some(d) = &v.delta {
                s += &format!(" {d}");
            }
            if let Some(a) = age {
                s += &format!(" · {a}");
            }
            s + demo
        }
        (None, Status::Loading) => format!("{}{demo}: loading", v.name),
        (None, _) => format!("{}{demo}: no recent reading", v.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Scenario;
    use crate::model::{Thresholds, Units};

    fn person() -> Person {
        Person {
            id: "x".into(),
            name: "Alex".into(),
            initials: "AL".into(),
            color: "#000".into(),
            units: Units::Mgdl,
            thresholds: Thresholds::default(),
            source: SourceConfig::Demo {
                scenario: Scenario::Normal,
            },
            tray_icon: true,
        }
    }

    #[test]
    fn hides_value_when_old() {
        let now = 1_700_000_000_000;
        let rt = Runtime {
            readings: vec![Reading {
                mgdl: 140,
                trend: Trend::Flat,
                time_ms: now - 25 * 60_000,
            }],
            error: None,
        };
        let v = person_view(&person(), Some(&rt), now);
        assert_eq!(v.status, Status::NoData);
        assert!(v.value.is_none());
    }

    #[test]
    fn sign_in_overrides() {
        let now = 1_700_000_000_000;
        let rt = Runtime {
            readings: vec![Reading {
                mgdl: 140,
                trend: Trend::Flat,
                time_ms: now - 60_000,
            }],
            error: Some((Status::SignIn, "bad password".into())),
        };
        let v = person_view(&person(), Some(&rt), now);
        assert_eq!(v.status, Status::SignIn);
        assert!(v.value.is_none());
    }

    #[test]
    fn delta_and_summary() {
        let now = 1_700_000_000_000;
        let rt = Runtime {
            readings: vec![
                Reading {
                    mgdl: 140,
                    trend: Trend::Flat,
                    time_ms: now - 7 * 60_000,
                },
                Reading {
                    mgdl: 142,
                    trend: Trend::Flat,
                    time_ms: now - 2 * 60_000,
                },
            ],
            error: None,
        };
        let v = person_view(&person(), Some(&rt), now);
        assert_eq!(v.delta.as_deref(), Some("+2"));
        assert_eq!(summary_line(&v), "Alex 142 mg/dL → +2 · 2 min ago [demo]");
    }
}
