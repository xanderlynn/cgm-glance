//! One background task per person that fetches readings on the sensor's schedule.

use crate::config::SourceConfig;
use crate::model::{now_ms, Status};
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

pub fn stop_all(app: &AppHandle) {
    let handles: Vec<_> = app
        .state::<AppState>()
        .lock()
        .pollers
        .drain()
        .map(|(_, h)| h)
        .collect();
    for h in handles {
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
    let secret = match &person.source {
        SourceConfig::Demo { .. } => None,
        _ => match secrets::load(&person.id) {
            Ok(s) => Some(s),
            Err(e) => {
                set_error(app, id, Status::SignIn, e);
                return;
            }
        },
    };
    let seed = person.id.bytes().map(|b| b as u64).sum();
    let client = match SourceClient::build(&person.source, secret, state.http.clone(), fast, seed) {
        Ok(c) => c,
        Err(e) => {
            set_error(app, id, Status::SignIn, e);
            return;
        }
    };
    let app2 = app.clone();
    let id2 = id.to_string();
    let handle = tauri::async_runtime::spawn(async move { run(app2, id2, client).await });
    state.lock().pollers.insert(id.to_string(), handle);
}

fn set_error(app: &AppHandle, id: &str, status: Status, msg: String) {
    app.state::<AppState>()
        .lock()
        .runtime
        .entry(id.to_string())
        .or_insert_with(Runtime::default)
        .error = Some((status, msg));
    ui::refresh(app);
}

async fn run(app: AppHandle, id: String, mut client: SourceClient) {
    let mut backoff = 30u64;
    loop {
        let result = client.fetch().await;
        let delay = {
            let state = app.state::<AppState>();
            let mut inner = state.lock();
            let rt = inner.runtime.entry(id.clone()).or_default();
            match result {
                Ok(readings) => {
                    rt.error = None;
                    if !readings.is_empty() {
                        rt.readings = readings;
                    }
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
    app.state::<AppState>().lock().pollers.remove(&id);
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
