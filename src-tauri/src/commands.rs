//! Commands the windows can call. Secrets arrive once (in `test_connection`), stay in
//! memory as a draft, and go straight to the OS keychain on `save_person`.
//! No command ever returns a secret to a window.

use crate::config::{Person, Region, Scenario, Settings, SourceConfig};
use crate::model::{self, Thresholds, Units};
use crate::secrets::{self, Secret};
use crate::sources::{nightscout, SourceClient};
use crate::state::{self, AppState, Draft, Snapshot};
use crate::{poller, ui};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NewSource {
    DexcomShare {
        region: Region,
        username: String,
        password: String,
    },
    Nightscout {
        url: String,
        token: Option<String>,
    },
    Demo {
        scenario: Scenario,
    },
}

#[derive(Serialize)]
pub struct TestResult {
    pub draft_id: String,
    pub source_label: String,
    pub value: Option<String>,
    pub mgdl: Option<u16>,
    pub arrow: String,
    pub age_min: Option<i64>,
    pub note: Option<String>,
}

#[derive(Deserialize)]
pub struct Display {
    pub name: String,
    pub initials: String,
    pub color: String,
    pub units: Units,
    pub thresholds: Thresholds,
    pub tray_icon: bool,
}

fn validate(d: &Display) -> Result<(), String> {
    let name = d.name.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err("Enter a name up to 40 characters.".into());
    }
    let n = d.initials.trim().chars().count();
    if n == 0 || n > 3 {
        return Err("Initials must be 1 to 3 characters.".into());
    }
    let c = d.color.as_bytes();
    if c.len() != 7 || c[0] != b'#' || !c[1..].iter().all(|b| b.is_ascii_hexdigit()) {
        return Err("Pick a color.".into());
    }
    let t = &d.thresholds;
    if !(40 <= t.urgent_low && t.urgent_low < t.low && t.low < t.high && t.high <= 400) {
        return Err("Ranges must go urgent low < low < high, between 40 and 400 mg/dL.".into());
    }
    Ok(())
}

fn apply_display(p: &mut Person, d: Display) {
    p.name = d.name.trim().to_string();
    p.initials = d.initials.trim().to_uppercase();
    p.color = d.color;
    p.units = d.units;
    p.thresholds = d.thresholds;
    p.tray_icon = d.tray_icon;
}

#[tauri::command]
pub fn get_snapshot(app: AppHandle) -> Snapshot {
    state::snapshot(&app.state::<AppState>())
}

#[tauri::command]
pub async fn test_connection(app: AppHandle, source: NewSource) -> Result<TestResult, String> {
    let state = app.state::<AppState>();
    let (cfg, secret) = match source {
        NewSource::DexcomShare {
            region,
            username,
            password,
        } => {
            if username.trim().is_empty() || password.is_empty() {
                return Err("Enter the Dexcom username and password.".into());
            }
            (
                SourceConfig::DexcomShare { region },
                Some(Secret::DexcomShare {
                    username: username.trim().to_string(),
                    password,
                }),
            )
        }
        NewSource::Nightscout { url, token } => {
            let url = nightscout::normalize_url(&url)?;
            let token = token
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
            (
                SourceConfig::Nightscout { url },
                Some(Secret::Nightscout { token }),
            )
        }
        NewSource::Demo { scenario } => (SourceConfig::Demo { scenario }, None),
    };
    let fast = state.lock().config.settings.demo_fast;
    let seed = model::now_ms() as u64;
    let mut client = SourceClient::build(&cfg, secret.clone(), state.http.clone(), fast, seed)?;
    // One attempt only. A wrong Dexcom password is never retried automatically.
    let (readings, note) = match client.fetch().await {
        Ok(r) => (r, None),
        Err(e) if cfg.is_demo() => (
            vec![],
            Some(format!(
                "{} This demo person will show that state.",
                e.message()
            )),
        ),
        Err(e) => return Err(e.message().to_string()),
    };
    let latest = readings.last();
    let now = model::now_ms();
    let draft_id = uuid::Uuid::new_v4().to_string();
    {
        let mut inner = state.lock();
        inner
            .drafts
            .retain(|_, d| d.created.elapsed() < Duration::from_secs(15 * 60));
        inner.drafts.insert(
            draft_id.clone(),
            Draft {
                source: cfg.clone(),
                secret,
                created: Instant::now(),
            },
        );
    }
    Ok(TestResult {
        draft_id,
        source_label: cfg.label(),
        value: latest.map(|r| model::format_value(r.mgdl, Units::Mgdl)),
        mgdl: latest.map(|r| r.mgdl),
        arrow: latest
            .map(|r| r.trend.arrow().to_string())
            .unwrap_or_default(),
        age_min: latest.map(|r| model::age_minutes(r, now)),
        note: note.or_else(|| {
            latest
                .is_none()
                .then(|| "Connected, but there are no readings from the last 3 hours.".into())
        }),
    })
}

/// Saves a new person, or (with `replace_id`) new sign-in details for an existing one.
#[tauri::command]
pub fn save_person(
    app: AppHandle,
    draft_id: String,
    display: Display,
    replace_id: Option<String>,
) -> Result<String, String> {
    validate(&display)?;
    let state = app.state::<AppState>();
    let draft = state
        .lock()
        .drafts
        .remove(&draft_id)
        .ok_or("That connection test has expired. Test the connection again.")?;
    let id = replace_id.unwrap_or_else(|| {
        let u = uuid::Uuid::new_v4().to_string();
        if draft.source.is_demo() {
            format!("demo-{u}")
        } else {
            u
        }
    });
    match &draft.secret {
        Some(secret) => secrets::store(&id, secret)?,
        None => secrets::delete(&id),
    }
    {
        let mut inner = state.lock();
        match inner.config.people.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.source = draft.source;
                apply_display(p, display);
            }
            None => {
                let mut p = Person {
                    id: id.clone(),
                    name: String::new(),
                    initials: String::new(),
                    color: String::new(),
                    units: Units::Mgdl,
                    thresholds: Thresholds::default(),
                    source: draft.source,
                    tray_icon: true,
                };
                apply_display(&mut p, display);
                inner.config.people.push(p);
            }
        }
        inner.runtime.remove(&id);
        state.save(&inner);
    }
    poller::start(&app, &id);
    ui::refresh(&app);
    Ok(id)
}

#[tauri::command]
pub fn update_person(app: AppHandle, id: String, display: Display) -> Result<(), String> {
    validate(&display)?;
    let state = app.state::<AppState>();
    {
        let mut inner = state.lock();
        let p = inner
            .config
            .people
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("Person not found.")?;
        apply_display(p, display);
        state.save(&inner);
    }
    ui::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn remove_person(app: AppHandle, id: String) {
    poller::stop(&app, &id);
    secrets::delete(&id);
    let state = app.state::<AppState>();
    {
        let mut inner = state.lock();
        inner.config.people.retain(|p| p.id != id);
        inner.config.badge_positions.remove(&format!("badge-{id}"));
        inner.config.settings.badge.people.retain(|p| *p != id);
        if inner.config.settings.tray.primary.as_deref() == Some(id.as_str()) {
            inner.config.settings.tray.primary = None;
        }
        inner.runtime.remove(&id);
        state.save(&inner);
    }
    ui::refresh(&app);
}

#[tauri::command]
pub fn save_settings(app: AppHandle, mut settings: Settings) -> Result<(), String> {
    settings.badge.opacity = settings.badge.opacity.clamp(0.3, 1.0);
    let state = app.state::<AppState>();
    let (fast_changed, autostart) = {
        let mut inner = state.lock();
        let old = inner.config.settings.clone();
        let layout_changed = old.badge.position != settings.badge.position
            || old.badge.edge != settings.badge.edge
            || old.badge.corner != settings.badge.corner;
        if layout_changed {
            inner.badge_geometry.clear();
        }
        let fast_changed = old.demo_fast != settings.demo_fast;
        let autostart = (old.autostart != settings.autostart).then_some(settings.autostart);
        inner.config.settings = settings;
        state.save(&inner);
        (fast_changed, autostart)
    };
    if let Some(on) = autostart {
        use tauri_plugin_autostart::ManagerExt;
        let r = if on {
            app.autolaunch().enable()
        } else {
            app.autolaunch().disable()
        };
        r.map_err(|e| format!("Could not change start at login: {e}"))?;
    }
    if fast_changed {
        poller::stop_all(&app);
        poller::start_all(&app);
    }
    ui::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn reset_badge_positions(app: AppHandle) {
    let state = app.state::<AppState>();
    {
        let mut inner = state.lock();
        inner.config.badge_positions.clear();
        inner.badge_geometry.clear();
        state.save(&inner);
    }
    ui::refresh(&app);
}

#[tauri::command]
pub fn retry_person(app: AppHandle, id: String) {
    app.state::<AppState>().lock().runtime.remove(&id);
    poller::start(&app, &id);
    ui::refresh(&app);
}

#[tauri::command]
pub fn open_settings(app: AppHandle, tab: Option<String>) {
    ui::open_settings(&app, tab.as_deref());
}

#[tauri::command]
pub fn hide_popup(app: AppHandle) {
    if let Some(w) = app.get_webview_window("popup") {
        let _ = w.hide();
    }
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d() -> Display {
        Display {
            name: "Alex".into(),
            initials: "al".into(),
            color: "#4C6FFF".into(),
            units: Units::Mgdl,
            thresholds: Thresholds::default(),
            tray_icon: true,
        }
    }

    #[test]
    fn validation() {
        assert!(validate(&d()).is_ok());
        let mut x = d();
        x.color = "red".into();
        assert!(validate(&x).is_err());
        let mut x = d();
        x.thresholds.low = 200;
        assert!(validate(&x).is_err());
        let mut x = d();
        x.initials = "ABCD".into();
        assert!(validate(&x).is_err());
    }
}
