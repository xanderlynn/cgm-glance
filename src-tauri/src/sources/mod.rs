//! Where readings come from. Each source returns readings sorted oldest → newest.

pub mod demo;
pub mod nightscout;
pub mod share;

use crate::config::SourceConfig;
use crate::model::Reading;
use crate::secrets::Secret;

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

    pub async fn fetch(&mut self) -> Result<Vec<Reading>, FetchError> {
        match self {
            SourceClient::Share(c) => c.fetch().await,
            SourceClient::Nightscout(c) => c.fetch().await,
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
