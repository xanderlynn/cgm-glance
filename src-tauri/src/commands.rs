//! Commands the windows can call. Secrets arrive once (in `test_connection`), stay in
//! memory as a draft, and go straight to the OS keychain on `save_person`.
//! No command ever returns a secret to a window.

use crate::config::{Person, Region, Scenario, Settings, SourceConfig};
use crate::model::{self, Thresholds, Units};
use crate::secrets::{self, Secret};
use crate::sources::{nightscout, FetchError, SourceClient};
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
    /// Range of the latest reading with default ranges, so the wizard can color it.
    pub status: crate::model::Status,
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
    if !crate::config::is_color(&d.color) {
        return Err("Pick a color.".into());
    }
    let t = &d.thresholds;
    if !(40 <= t.urgent_low && t.urgent_low < t.low && t.low < t.high && t.high <= 400) {
        return Err("Ranges must go urgent low < low < high, between 40 and 400 mg/dL (2.2 and 22.2 mmol/L).".into());
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
    // Dexcom locks accounts after repeated wrong passwords, so space out test attempts.
    let is_share = matches!(cfg, SourceConfig::DexcomShare { .. });
    if is_share
        && state
            .lock()
            .last_auth_fail
            .is_some_and(|t| t.elapsed() < Duration::from_secs(20))
    {
        return Err(
            "Wait 20 seconds before trying again, so Dexcom doesn't lock the account.".into(),
        );
    }
    let mut client = SourceClient::build(&cfg, secret.clone(), state.http.clone(), fast, seed)?;
    // One attempt only. A wrong Dexcom password is never retried automatically.
    let (readings, note) = match client.fetch(None).await {
        Ok(r) => (r, None),
        Err(FetchError::Auth(m)) if is_share => {
            state.lock().last_auth_fail = Some(Instant::now());
            return Err(m);
        }
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
        status: model::classify(latest, now, &Thresholds::default()),
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
    let (source, secret) = {
        let inner = state.lock();
        let d = inner
            .drafts
            .get(&draft_id)
            .ok_or("That connection test has expired. Test the connection again.")?;
        (d.source.clone(), d.secret.clone())
    };
    let replacing = replace_id.is_some();
    let id = replace_id.unwrap_or_else(|| {
        let u = uuid::Uuid::new_v4().to_string();
        if source.is_demo() {
            format!("demo-{u}")
        } else {
            u
        }
    });
    // If the keychain fails, the draft stays so the user can retry without retyping.
    match &secret {
        Some(secret) => secrets::store(&id, secret)?,
        // Switching an existing person to demo data removes their old sign-in.
        None if replacing => secrets::delete(&id),
        None => {}
    }
    {
        let mut inner = state.lock();
        inner.drafts.remove(&draft_id);
        match inner.config.people.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.source = source;
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
                    source,
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
        inner.config.settings.alarms.off_for.retain(|p| *p != id);
        if inner.config.settings.tray.primary.as_deref() == Some(id.as_str()) {
            inner.config.settings.tray.primary = None;
        }
        inner.runtime.remove(&id);
        state.save(&inner);
    }
    ui::refresh(&app);
}

/// Signs a person out: deletes their saved password from the keychain, stops fetching and
/// drops their readings. Their name, colour and alarm settings stay, so "Sign in again"
/// brings them back.
#[tauri::command]
pub fn sign_out(app: AppHandle, id: String) {
    poller::stop(&app, &id);
    secrets::delete(&id);
    let state = app.state::<AppState>();
    {
        let mut inner = state.lock();
        if !inner.config.people.iter().any(|p| p.id == id) {
            return;
        }
        let rt = inner.runtime.entry(id).or_default();
        rt.readings.clear();
        rt.error = Some((
            crate::model::Status::SignIn,
            "Signed out. Use \"Sign in again\" to resume.".into(),
        ));
    }
    ui::refresh(&app);
}

/// Erases everything: every person and saved password, all settings and window
/// positions. The app is left as if freshly installed.
#[tauri::command]
pub fn erase_all_data(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (ids, autostart) = {
        let inner = state.lock();
        let ids: Vec<String> = inner.config.people.iter().map(|p| p.id.clone()).collect();
        (ids, inner.config.settings.autostart)
    };
    for id in &ids {
        poller::stop(&app, id);
        secrets::delete(id);
    }
    if autostart {
        use tauri_plugin_autostart::ManagerExt;
        let _ = app.autolaunch().disable();
    }
    {
        let mut inner = state.lock();
        inner.config = crate::config::Config::default();
        inner.runtime.clear();
        inner.drafts.clear();
        inner.engine = Default::default();
        // Remove the files themselves (including any kept unreadable copy), then write a
        // fresh default so nothing of the old settings is left on disk.
        let _ = std::fs::remove_file(state.path.with_extension("json.unreadable"));
        let _ = std::fs::remove_file(&state.path);
        crate::config::save(&state.path, &inner.config)
            .map_err(|e| format!("Could not reset the settings file: {e}"))?;
    }
    ui::refresh_force(&app);
    Ok(())
}

#[tauri::command]
pub fn save_settings(app: AppHandle, mut settings: Settings) -> Result<(), String> {
    settings.badge.opacity = settings.badge.opacity.clamp(0.3, 1.0);
    settings.alarms.normalize();
    let state = app.state::<AppState>();
    let old_autostart = state.lock().config.settings.autostart;
    if settings.autostart != old_autostart {
        use tauri_plugin_autostart::ManagerExt;
        let r = if settings.autostart {
            app.autolaunch().enable()
        } else {
            app.autolaunch().disable()
        };
        r.map_err(|e| format!("Could not change start at login: {e}"))?;
    }
    let fast_changed = {
        let mut inner = state.lock();
        let old = inner.config.settings.clone();
        let layout_changed = old.badge.position != settings.badge.position
            || old.badge.edge != settings.badge.edge
            || old.badge.corner != settings.badge.corner;
        if layout_changed {
            inner.badge_geometry.clear();
        }
        let fast_changed = old.demo_fast != settings.demo_fast;
        inner.config.settings = settings;
        state.save(&inner);
        fast_changed
    };
    if fast_changed {
        // Only demo people. Restarting a real source could retry a rejected password.
        let demo_ids: Vec<String> = state
            .lock()
            .config
            .people
            .iter()
            .filter(|p| p.source.is_demo())
            .map(|p| p.id.clone())
            .collect();
        for id in demo_ids {
            poller::start(&app, &id);
        }
    }
    ui::refresh(&app);
    Ok(())
}

/// Forgets tested sign-ins when the wizard is closed without saving.
#[tauri::command]
pub fn discard_drafts(app: AppHandle) {
    app.state::<AppState>().lock().drafts.clear();
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
    ui::refresh_force(&app);
}

#[tauri::command]
pub fn retry_person(app: AppHandle, id: String) {
    app.state::<AppState>().lock().runtime.remove(&id);
    poller::start(&app, &id);
    ui::refresh(&app);
}

#[tauri::command]
pub fn open_settings(app: AppHandle, tab: Option<String>) {
    // The tab goes into a URL and an event, so accept only known shapes.
    let tab = tab.filter(|t| valid_tab(t));
    ui::open_settings(&app, tab.as_deref());
}

fn valid_tab(t: &str) -> bool {
    const TABS: [&str; 7] = [
        "people", "tray", "badge", "alarms", "general", "about", "add",
    ];
    TABS.contains(&t)
        || t.strip_prefix("signin:").is_some_and(|id| {
            !id.is_empty()
                && id.len() <= 64
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
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

/// Acknowledges one person's alarm, or everyone's when `id` is None.
#[tauri::command]
pub fn acknowledge_alarms(app: AppHandle, id: Option<String>) {
    crate::alarms::with_engine(&app, |e, s, now| e.acknowledge(s, id.as_deref(), now));
    ui::refresh(&app);
}

/// Silences one person's alarms (or everyone's) for the configured snooze time.
#[tauri::command]
pub fn snooze_alarms(app: AppHandle, id: Option<String>) {
    crate::alarms::with_engine(&app, |e, s, now| e.snooze(s, id.as_deref(), now));
    ui::refresh(&app);
}

/// Starts a manual quiet period for `minutes`, or ends it with 0.
#[tauri::command]
pub fn set_quiet(app: AppHandle, minutes: u32) {
    crate::alarms::with_engine(&app, |e, _, now| e.set_quiet(minutes, now));
    ui::refresh(&app);
}

/// Checks GitHub Releases for a newer version now.
#[tauri::command]
pub fn check_update(app: AppHandle) {
    crate::updater::check(&app, true);
}

/// Downloads, verifies and installs the update found by the last check, then restarts.
#[tauri::command]
pub fn install_update(app: AppHandle) {
    crate::updater::install(&app);
}

#[cfg(test)]
mod acl_tests {
    //! Each window may call only the commands its capability grants
    //! (`capabilities/*.json`). These tests keep the UI and the grants in step.
    use std::collections::BTreeSet;

    fn granted(capability: &str) -> BTreeSet<String> {
        let path = format!(
            "{}/capabilities/{capability}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        v["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str()?.strip_prefix("allow-"))
            .map(|c| c.replace('-', "_"))
            .collect()
    }

    /// Commands a page calls, from `invoke("name"` in its script and common.js.
    fn used(page: &str) -> BTreeSet<String> {
        let ui = format!("{}/../ui", env!("CARGO_MANIFEST_DIR"));
        let mut out = BTreeSet::new();
        for file in [format!("{ui}/{page}.js"), format!("{ui}/common.js")] {
            let src = std::fs::read_to_string(file).unwrap();
            for part in src.split("invoke(\"").skip(1) {
                out.insert(part.split('"').next().unwrap().to_string());
            }
        }
        out
    }

    #[test]
    fn every_window_is_granted_what_it_uses() {
        for page in ["badge", "popup", "settings"] {
            let missing: Vec<_> = used(page).difference(&granted(page)).cloned().collect();
            assert!(
                missing.is_empty(),
                "{page} calls ungranted commands: {missing:?}"
            );
        }
    }

    #[test]
    fn settings_tab_names_are_checked() {
        assert!(super::valid_tab("alarms"));
        assert!(super::valid_tab("signin:demo-6bbd3df3-65ee-4403-9df3"));
        assert!(!super::valid_tab("signin:"));
        assert!(!super::valid_tab("x\"><img src=x>"));
        assert!(!super::valid_tab("signin:../../etc"));
    }

    #[test]
    fn badges_can_only_read() {
        assert_eq!(
            granted("badge"),
            BTreeSet::from(["get_snapshot".to_string()])
        );
    }

    #[test]
    fn only_settings_handles_sign_ins() {
        for page in ["badge", "popup"] {
            for c in [
                "test_connection",
                "save_person",
                "update_person",
                "remove_person",
            ] {
                assert!(!granted(page).contains(c), "{page} must not get {c}");
            }
        }
    }
}
