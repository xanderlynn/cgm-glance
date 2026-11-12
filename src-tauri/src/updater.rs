//! In-app updates from GitHub Releases (signed-manifest, client-pull).
//!
//! The release workflow uploads signed installers and a `latest.json` manifest to a
//! release. The app fetches that one file over HTTPS, and the Tauri updater installs a
//! download only if its signature matches the public key built into the app
//! (`tauri.conf.json` → `plugins.updater.pubkey`). Draft releases aren't served, so
//! publishing a release is the manual step that ships an update.
//!
//! State machine, owned here and shown through the snapshot:
//! Idle → Checking → UpToDate | Available | Error → Downloading → Installing → restart.

use crate::state::AppState;
use crate::{model, ui};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Error, Update, UpdaterExt};

/// First automatic check, after start-up has settled. `CGM_GLANCE_UPDATE_DELAY_SECS`
/// overrides it (for testing the real check without waiting).
fn first_check() -> Duration {
    let secs = std::env::var("CGM_GLANCE_UPDATE_DELAY_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);
    Duration::from_secs(secs)
}
const EVERY: Duration = Duration::from_secs(24 * 3600);

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate {
        checked_ms: i64,
    },
    /// No release has been published yet (nothing to compare against).
    NoRelease {
        checked_ms: i64,
    },
    Available {
        version: String,
        notes: Option<String>,
    },
    Downloading {
        percent: Option<u8>,
    },
    Installing,
    Error {
        message: String,
    },
}

impl Status {
    fn busy(&self) -> bool {
        matches!(
            self,
            Status::Checking | Status::Downloading { .. } | Status::Installing
        )
    }
}

#[derive(Default)]
pub struct UpdateState {
    pub status: Status,
    /// The update found by the last check, ready to install.
    pending: Option<Update>,
}

/// What the windows show.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct UpdateView {
    pub current: String,
    pub status: Status,
}

pub fn view(s: &UpdateState) -> UpdateView {
    UpdateView {
        current: env!("CARGO_PKG_VERSION").to_string(),
        status: s.status.clone(),
    }
}

fn set(app: &AppHandle, status: Status) {
    app.state::<AppState>().lock().update.status = status;
    ui::refresh(app);
}

/// Release notes can be long; the windows show a short excerpt.
fn excerpt(notes: Option<String>) -> Option<String> {
    let notes = notes?.trim().to_string();
    if notes.is_empty() {
        return None;
    }
    Some(if notes.chars().count() > 600 {
        notes.chars().take(599).collect::<String>() + "…"
    } else {
        notes
    })
}

/// Checks for an update. A manual check reports errors; an automatic one stays quiet
/// about them so a laptop offline at the time doesn't get an error banner.
pub fn check(app: &AppHandle, manual: bool) {
    {
        let state = app.state::<AppState>();
        let mut inner = state.lock();
        if inner.update.status.busy() {
            return;
        }
        if manual {
            inner.update.status = Status::Checking;
        }
    }
    ui::refresh(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = match app.updater() {
            Ok(u) => u.check().await,
            Err(e) => Err(e),
        };
        let now = model::now_ms();
        let state = app.state::<AppState>();
        {
            let mut inner = state.lock();
            let previous = inner.update.status.clone();
            inner.update.status = match result {
                Ok(Some(update)) => {
                    let status = Status::Available {
                        version: update.version.clone(),
                        notes: excerpt(update.body.clone()),
                    };
                    inner.update.pending = Some(update);
                    status
                }
                Ok(None) => Status::UpToDate { checked_ms: now },
                // GitHub answers 404 until the first release is published.
                Err(Error::ReleaseNotFound) => Status::NoRelease { checked_ms: now },
                Err(e) if manual => Status::Error {
                    message: format!("Couldn't check for updates: {e}"),
                },
                Err(e) => {
                    eprintln!("cgm-glance: update check failed: {e}");
                    // Keep showing a found update, or go back to idle.
                    match previous {
                        Status::Available { .. } => previous,
                        _ => Status::Idle,
                    }
                }
            };
            eprintln!("cgm-glance: update check: {:?}", inner.update.status);
        }
        ui::refresh(&app);
    });
}

/// Downloads and installs the update found by the last check, then restarts.
pub fn install(app: &AppHandle) {
    let pending = {
        let state = app.state::<AppState>();
        let mut inner = state.lock();
        if inner.update.status.busy() {
            return;
        }
        inner.update.pending.take()
    };
    let Some(update) = pending else {
        set(
            app,
            Status::Error {
                message: "No update is ready. Check for updates first.".into(),
            },
        );
        return;
    };
    set(app, Status::Downloading { percent: None });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut got: u64 = 0;
        let mut shown: Option<u8> = None;
        let app2 = app.clone();
        let app3 = app.clone();
        let result = update
            .download_and_install(
                |chunk, total| {
                    got += chunk as u64;
                    let percent = total
                        .filter(|&t| t > 0)
                        .map(|t| (got * 100 / t).min(100) as u8);
                    // Refresh only when the shown percentage changes.
                    if percent != shown {
                        shown = percent;
                        set(&app2, Status::Downloading { percent });
                    }
                },
                || set(&app3, Status::Installing),
            )
            .await;
        match result {
            // On Windows the installer has already taken over; elsewhere restart into
            // the new version.
            Ok(()) => app.restart(),
            Err(e) => set(
                &app,
                Status::Error {
                    message: format!("The update didn't install: {e}. Nothing was changed."),
                },
            ),
        }
    });
}

/// Checks shortly after start, then daily, while automatic checks are on.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(first_check()).await;
        loop {
            let auto = app.state::<AppState>().lock().config.settings.auto_update;
            if auto {
                check(&app, false);
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_trimmed_and_shortened() {
        assert_eq!(excerpt(None), None);
        assert_eq!(excerpt(Some("  \n ".into())), None);
        assert_eq!(excerpt(Some(" Fixes \n".into())).as_deref(), Some("Fixes"));
        let long = excerpt(Some("x".repeat(2000))).unwrap();
        assert_eq!(long.chars().count(), 600);
    }

    #[test]
    fn status_serializes_with_a_state_tag() {
        let s = serde_json::to_value(Status::Downloading { percent: Some(40) }).unwrap();
        assert_eq!(s["state"], "downloading");
        assert_eq!(s["percent"], 40);
        assert!(Status::Installing.busy() && !Status::Idle.busy());
    }

    #[test]
    fn config_points_at_a_signed_https_manifest() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let up = &conf["plugins"]["updater"];
        let pubkey = up["pubkey"].as_str().unwrap();
        assert!(pubkey.len() > 40, "public key is set");
        // A private key must never be committed: minisign secret keys say so in their header.
        let decoded = String::from_utf8(base64_decode(pubkey).expect("pubkey is base64")).unwrap();
        assert!(decoded.contains("public key"), "{decoded}");
        for e in up["endpoints"].as_array().unwrap() {
            assert!(e.as_str().unwrap().starts_with("https://github.com/"));
        }
        assert_eq!(conf["bundle"]["createUpdaterArtifacts"], true);
    }

    /// Minimal base64 decoder for the test (avoids a dependency).
    fn base64_decode(s: &str) -> Option<Vec<u8>> {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = vec![];
        let mut buf = 0u32;
        let mut bits = 0;
        for c in s.bytes().filter(|&c| c != b'=') {
            buf = buf << 6 | T.iter().position(|&t| t == c)? as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
            }
        }
        Some(out)
    }
}
