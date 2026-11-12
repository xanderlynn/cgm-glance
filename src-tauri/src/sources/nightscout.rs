//! Nightscout client. Uses a read-only access token; no Dexcom password involved.

use super::{network_error, FetchError};
use crate::model::{Reading, Trend};
use serde_json::Value;

pub struct NightscoutClient {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

/// Accepts "https://my-site.example" with or without a trailing slash. HTTPS only.
pub fn normalize_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    if !url.starts_with("https://") || url.len() <= "https://".len() {
        return Err("Enter the full Nightscout address starting with https://".into());
    }
    if url.contains('?') || url.contains('#') || url.contains('@') {
        return Err("Enter only the site address. Put the token in the token field.".into());
    }
    Ok(url.to_string())
}

impl NightscoutClient {
    pub fn new(http: reqwest::Client, url: &str, token: Option<String>) -> Result<Self, String> {
        Ok(NightscoutClient {
            http,
            base: normalize_url(url)?,
            token: token.filter(|t| !t.trim().is_empty()),
        })
    }

    pub async fn fetch(&mut self) -> Result<Vec<Reading>, FetchError> {
        let mut req = self
            .http
            .get(format!("{}/api/v1/entries/sgv.json", self.base))
            .query(&[("count", "40")]);
        if let Some(t) = &self.token {
            req = req.query(&[("token", t.as_str())]);
        }
        let resp = req
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(network_error)?;
        match resp.status().as_u16() {
            200 => {}
            401 | 403 => {
                return Err(FetchError::Auth(
                    "Nightscout refused the access token. Check it has the readable role.".into(),
                ))
            }
            404 => {
                return Err(FetchError::Other(
                    "No Nightscout site found at that address.".into(),
                ))
            }
            s => {
                return Err(FetchError::Other(format!(
                    "Nightscout answered with error {s}."
                )))
            }
        }
        let v: Value = resp.json().await.map_err(network_error)?;
        parse_entries(&v)
    }
}

/// Parses `[{"sgv":142,"date":1700000600000,"direction":"Flat"}, ...]`.
pub fn parse_entries(v: &Value) -> Result<Vec<Reading>, FetchError> {
    let arr = v
        .as_array()
        .ok_or_else(|| FetchError::Other("Unexpected answer from Nightscout.".into()))?;
    let mut out: Vec<Reading> = arr
        .iter()
        .filter_map(|e| {
            let mgdl = e.get("sgv")?.as_f64()?.round() as u16;
            let time_ms = e.get("date")?.as_f64()? as i64;
            let trend = e
                .get("direction")
                .and_then(Value::as_str)
                .map(Trend::from_name)
                .unwrap_or(Trend::Unknown);
            Some(Reading {
                mgdl,
                trend,
                time_ms,
            })
        })
        .collect();
    out.sort_by_key(|r| r.time_ms);
    out.dedup_by_key(|r| r.time_ms);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            normalize_url(" https://ns.example.org/ ").unwrap(),
            "https://ns.example.org"
        );
        assert!(normalize_url("http://ns.example.org").is_err());
        assert!(normalize_url("https://ns.example.org?token=abc").is_err());
        assert!(normalize_url("https://").is_err());
    }

    #[test]
    fn parses_entries() {
        // Synthetic entries in Nightscout's format.
        let v: Value = serde_json::from_str(
            r#"[{"sgv":88,"date":1700000600000,"direction":"FortyFiveDown"},
                {"sgv":94,"date":1700000300000,"direction":"Flat"},
                {"type":"mbg","mbg":100,"date":1700000100000}]"#,
        )
        .unwrap();
        let r = parse_entries(&v).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].mgdl, 88);
        assert_eq!(r[1].trend, Trend::FortyFiveDown);
    }
}
