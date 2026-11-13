//! One background task per person that fetches readings on the sensor's schedule.

use crate::config::SourceConfig;
use crate::model::{merge_readings, now_ms, Status};
use crate::secrets;
use crate::sources::{FetchError, SourceClient};
use crate::state::{AppState, Runtime};
use crate::ui;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// Next fetch: 5 min 10 s after the last reading's timestamp, so we ask right after a new
/// reading should exist. If one is late, check every minute.
pub fn next_delay(last_ms: Option<i64>, now: i64) -> Duration {
    let Some(last) = last_ms else {
        return Duration::from_secs(60);
    };
    let due = last + 5 * 60_000 + 10_000;
    let wait_ms = if due > now { due - now } else { 60_000 };
    Duration::from_millis(wait_ms.clamp(15_000, 330_000) as u64)
}

pub fn start_all(app: &AppHandle) {
    let ids: Vec<String> = app
        .state::<AppState>()
        .lock()
        .config
        .people
        .iter()
        .map(|p| p.id.clone())
        .collect();
    for id in ids {
        start(app, &id);
    }
}

pub fn stop(app: &AppHandle, id: &str) {
    if let Some(h) = app.state::<AppState>().lock().pollers.remove(id) {
        h.abort();
    }
}

pub fn start(app: &AppHandle, id: &str) {
    stop(app, id);
    let state = app.state::<AppState>();
    let (person, fast) = {
        let inner = state.lock();
        let Some(p) = inner.config.people.iter().find(|p| p.id == id).cloned() else {
            return;
        };
        (p, inner.config.settings.demo_fast)
    };
    let http = state.http.clone();
    let app2 = app.clone();
    let id2 = id.to_string();
    let handle = tauri::async_runtime::spawn(async move {
        // Reading the keychain can wait on a macOS permission prompt, so it runs off the
        // UI thread: the tray and windows keep working while the prompt is open.
        let secret = match &person.source {
            SourceConfig::Demo { .. } => None,
            _ => {
                let pid = person.id.clone();
                match tauri::async_runtime::spawn_blocking(move || secrets::load(&pid)).await {
                    Ok(Ok(s)) => Some(s),
                    Ok(Err(e)) => return set_error(&app2, &id2, Status::SignIn, e),
                    Err(e) => return set_error(&app2, &id2, Status::Error, e.to_string()),
                }
            }
        };
        let seed = person.id.bytes().map(|b| b as u64).sum();
        let client = match SourceClient::build(&person.source, secret, http, fast, seed) {
            Ok(c) => c,
            Err(e) => return set_error(&app2, &id2, Status::SignIn, e),
        };
        if debug_log() {
            eprintln!("cgm-glance: poller {} started", &id2[..id2.len().min(8)]);
        }
        run(app2, id2, client).await
    });
    state.lock().pollers.insert(id.to_string(), handle);
}

fn set_error(app: &AppHandle, id: &str, status: Status, msg: String) {
    if debug_log() {
        eprintln!(
            "cgm-glance: poller {} not started: {msg}",
            &id[..id.len().min(8)]
        );
    }
    app.state::<AppState>()
        .lock()
        .runtime
        .entry(id.to_string())
        .or_insert_with(Runtime::default)
        .error = Some((status, msg));
    ui::refresh(app);
}

/// `CGM_GLANCE_DEBUG=1` logs each poll's outcome to the terminal, for troubleshooting.
fn debug_log() -> bool {
    std::env::var("CGM_GLANCE_DEBUG").is_ok_and(|v| v == "1")
}

async fn run(app: AppHandle, id: String, mut client: SourceClient) {
    let mut backoff = 30u64;
    loop {
        let since = app
            .state::<AppState>()
            .lock()
            .runtime
            .get(&id)
            .and_then(|r| r.readings.last().map(|r| r.time_ms));
        let result = client.fetch(since).await;
        if debug_log() {
            // Counts and ages only: no names, values or credentials in the log.
            let now = now_ms();
            match &result {
                Ok(r) => eprintln!(
                    "cgm-glance: poll {}: {} readings, newest {} min old",
                    &id[..id.len().min(8)],
                    r.len(),
                    r.iter()
                        .map(|x| x.time_ms)
                        .max()
                        .map_or("-".into(), |t| ((now - t) / 60_000).to_string())
                ),
                Err(e) => eprintln!(
                    "cgm-glance: poll {}: error: {}",
                    &id[..id.len().min(8)],
                    e.message()
                ),
            }
        }
        let delay = {
            let state = app.state::<AppState>();
            let mut inner = state.lock();
            let rt = inner.runtime.entry(id.clone()).or_default();
            match result {
                Ok(readings) => {
                    rt.error = None;
                    rt.readings =
                        merge_readings(std::mem::take(&mut rt.readings), readings, now_ms());
                    backoff = 30;
                    Some(match client.fast_interval() {
                        Some(s) => Duration::from_secs(s),
                        None => next_delay(rt.readings.last().map(|r| r.time_ms), now_ms()),
                    })
                }
                Err(FetchError::Auth(m)) => {
                    // Stop. Retrying a bad password can lock the Dexcom account.
                    rt.error = Some((Status::SignIn, m));
                    None
                }
                Err(e) => {
                    rt.error = Some((Status::Error, e.message().to_string()));
                    let d = backoff;
                    backoff = (backoff * 2).min(300);
                    Some(Duration::from_secs(client.fast_interval().unwrap_or(d)))
                }
            }
        };
        ui::refresh(&app);
        match delay {
            Some(d) => tokio::time::sleep(d).await,
            None => break,
        }
    }
    // The finished handle stays in `pollers`; aborting a finished task is harmless, and
    // removing it here could remove a newer poller started for the same person.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_next_reading() {
        let now = 1_700_000_000_000;
        assert_eq!(
            next_delay(Some(now - 60_000), now),
            Duration::from_millis(250_000)
        );
        assert_eq!(
            next_delay(Some(now - 10 * 60_000), now),
            Duration::from_secs(60)
        );
        assert_eq!(next_delay(None, now), Duration::from_secs(60));
        assert_eq!(
            next_delay(Some(now - 309_000), now),
            Duration::from_secs(15)
        );
    }
}
