//! Where readings come from. Each source returns readings sorted oldest → newest.

pub mod demo;
pub mod nightscout;
pub mod share;

use crate::config::SourceConfig;
use crate::model::Reading;
use crate::secrets::Secret;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_window_shrinks_after_first_poll() {
        let now = 1_700_000_000_000;
        assert_eq!(fetch_window(None, now), (180, 38));
        assert_eq!(fetch_window(Some(now - 5 * 60_000), now), (15, 5));
        assert_eq!(fetch_window(Some(now - 10 * 3_600_000), now), (180, 38));
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FetchError {
    /// Wrong password / token. Polling stops; the user must sign in again.
    Auth(String),
    /// Network trouble. Retried with backoff.
    Network(String),
    /// Anything else. Retried with backoff.
    Other(String),
}

impl FetchError {
    pub fn message(&self) -> &str {
        match self {
            FetchError::Auth(m) | FetchError::Network(m) | FetchError::Other(m) => m,
        }
    }
}

/// Largest answer we accept. Real answers are a few KB; this stops a broken or hostile
/// server from filling memory.
const MAX_BODY: usize = 512 * 1024;

/// Reads a response body, refusing anything larger than `MAX_BODY`.
pub async fn read_body(mut resp: reqwest::Response) -> Result<Vec<u8>, FetchError> {
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(network_error)? {
        if buf.len() + chunk.len() > MAX_BODY {
            return Err(FetchError::Other(
                "The server sent an unexpectedly large answer.".into(),
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// How far back to ask for readings: the full 3 hours the first time, then only what's
/// new since the last reading we have (plus a margin), so each poll is small.
pub fn fetch_window(since_ms: Option<i64>, now_ms: i64) -> (u32, u32) {
    let minutes = match since_ms {
        Some(t) => (((now_ms - t).max(0) / 60_000) as u32 + 10).clamp(10, 180),
        None => 180,
    };
    (minutes, minutes / 5 + 2)
}

pub fn network_error(e: reqwest::Error) -> FetchError {
    if e.is_timeout() {
        FetchError::Network("The server took too long to answer.".into())
    } else if e.is_connect() {
        FetchError::Network("Can't connect. Check the internet connection.".into())
    } else {
        // reqwest errors can include the URL; strip it so no query values reach the UI or logs.
        FetchError::Network(format!("Network error: {}", e.without_url()))
    }
}

pub enum SourceClient {
    Share(share::ShareClient),
    Nightscout(nightscout::NightscoutClient),
    Demo(demo::DemoSource),
}

impl SourceClient {
    pub fn build(
        cfg: &SourceConfig,
        secret: Option<Secret>,
        http: reqwest::Client,
        demo_fast: bool,
        seed: u64,
    ) -> Result<Self, String> {
        match (cfg, secret) {
            (SourceConfig::Demo { scenario }, _) => Ok(SourceClient::Demo(demo::DemoSource::new(
                *scenario, demo_fast, seed,
            ))),
            (
                SourceConfig::DexcomShare { region },
                Some(Secret::DexcomShare { username, password }),
            ) => Ok(SourceClient::Share(share::ShareClient::new(
                http, *region, username, password,
            ))),
            (SourceConfig::Nightscout { url }, Some(Secret::Nightscout { token })) => Ok(
                SourceClient::Nightscout(nightscout::NightscoutClient::new(http, url, token)?),
            ),
            _ => Err("Saved sign-in does not match this source. Sign in again.".into()),
        }
    }

    /// `since_ms`: timestamp of the newest reading already held, if any.
    pub async fn fetch(&mut self, since_ms: Option<i64>) -> Result<Vec<Reading>, FetchError> {
        let window = fetch_window(since_ms, crate::model::now_ms());
        match self {
            SourceClient::Share(c) => c.fetch(window).await,
            SourceClient::Nightscout(c) => c.fetch(window.1).await,
            SourceClient::Demo(c) => c.fetch(),
        }
    }

    /// Seconds between polls when this is a fast demo source.
    pub fn fast_interval(&self) -> Option<u64> {
        match self {
            SourceClient::Demo(d) if d.fast => Some(demo::FAST_STEP_SECS),
            _ => None,
        }
    }
}
