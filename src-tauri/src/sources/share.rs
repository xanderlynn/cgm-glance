//! Dexcom Share client (the unofficial API behind the Dexcom Follow app).
//! Needs the login of the person wearing the sensor, not a follower.
//! On a password error we stop instead of retrying, so we never lock the account.

use super::{network_error, read_body, FetchError};
use crate::config::Region;
use crate::model::{Reading, Trend};
use serde_json::{json, Value};

const NULL_ID: &str = "00000000-0000-0000-0000-000000000000";
const APP_ID: &str = "d89443d2-327c-4a6f-89e5-496bbb0317db";
const APP_ID_JP: &str = "d8665ade-9673-4e27-9ff6-92db4ce13d13";

fn base_url(region: Region) -> &'static str {
    match region {
        Region::Us => "https://share2.dexcom.com/ShareWebServices/Services/",
        Region::Ous => "https://shareous1.dexcom.com/ShareWebServices/Services/",
        Region::Jp => "https://share.dexcom.jp/ShareWebServices/Services/",
    }
}

pub struct ShareClient {
    http: reqwest::Client,
    base: &'static str,
    app_id: &'static str,
    username: String,
    password: String,
    /// Kept in memory only.
    session: Option<String>,
}

enum CallError {
    SessionExpired,
    Fetch(FetchError),
}

impl From<FetchError> for CallError {
    fn from(e: FetchError) -> Self {
        CallError::Fetch(e)
    }
}

impl ShareClient {
    pub fn new(http: reqwest::Client, region: Region, username: String, password: String) -> Self {
        ShareClient {
            http,
            base: base_url(region),
            app_id: if region == Region::Jp {
                APP_ID_JP
            } else {
                APP_ID
            },
            username,
            password,
            session: None,
        }
    }

    async fn post(
        &self,
        endpoint: &str,
        body: Value,
        query: &[(&str, &str)],
    ) -> Result<Value, CallError> {
        let resp = self
            .http
            .post(format!("{}{}", self.base, endpoint))
            .query(query)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(network_error)?;
        let status = resp.status();
        let body = read_body(resp).await?;
        let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        if status.is_success() {
            Ok(value)
        } else {
            Err(classify_error(&value, status.as_u16()))
        }
    }

    async fn login(&mut self) -> Result<(), FetchError> {
        let bad = || {
            FetchError::Auth(
                "Dexcom did not accept this username and password, or the region is wrong.".into(),
            )
        };
        let unwrap = |r: Result<Value, CallError>| match r {
            Ok(v) => Ok(v),
            Err(CallError::SessionExpired) => Err(bad()),
            Err(CallError::Fetch(e)) => Err(e),
        };
        let account = unwrap(
            self.post(
                "General/AuthenticatePublisherAccount",
                json!({ "accountName": self.username, "password": self.password, "applicationId": self.app_id }),
                &[],
            )
            .await,
        )?;
        let account = account
            .as_str()
            .filter(|s| *s != NULL_ID)
            .ok_or_else(bad)?
            .to_string();
        let session = unwrap(
            self.post(
                "General/LoginPublisherAccountById",
                json!({ "accountId": account, "password": self.password, "applicationId": self.app_id }),
                &[],
            )
            .await,
        )?;
        let session = session
            .as_str()
            .filter(|s| *s != NULL_ID)
            .ok_or_else(bad)?
            .to_string();
        self.session = Some(session);
        Ok(())
    }

    async fn read(&self, (minutes, max_count): (u32, u32)) -> Result<Vec<Reading>, CallError> {
        let (minutes, max_count) = (minutes.to_string(), max_count.to_string());
        let sid = self.session.as_deref().unwrap_or(NULL_ID);
        let v = self
            .post(
                "Publisher/ReadPublisherLatestGlucoseValues",
                json!({}),
                &[
                    ("sessionId", sid),
                    ("minutes", &minutes),
                    ("maxCount", &max_count),
                ],
            )
            .await?;
        Ok(parse_readings(&v)?)
    }

    pub async fn fetch(&mut self, window: (u32, u32)) -> Result<Vec<Reading>, FetchError> {
        if self.session.is_none() {
            self.login().await?;
        }
        match self.read(window).await {
            Ok(r) => Ok(r),
            Err(CallError::SessionExpired) => {
                // Sessions expire after a while; sign in once more, then give up.
                self.session = None;
                self.login().await?;
                match self.read(window).await {
                    Ok(r) => Ok(r),
                    Err(CallError::SessionExpired) => Err(FetchError::Other(
                        "Dexcom keeps rejecting the session.".into(),
                    )),
                    Err(CallError::Fetch(e)) => Err(e),
                }
            }
            Err(CallError::Fetch(e)) => Err(e),
        }
    }
}

fn classify_error(v: &Value, status: u16) -> CallError {
    let code = v.get("Code").and_then(Value::as_str).unwrap_or("");
    match code {
        "SessionIdNotFound" | "SessionNotValid" => CallError::SessionExpired,
        c if c.contains("MaxAttempts") => CallError::Fetch(FetchError::Auth(
            "Dexcom has temporarily locked sign-in after too many attempts. Wait, then sign in again.".into(),
        )),
        c if c.contains("Password") || c.contains("AccountNotFound") || c.contains("InvalidArgument") => {
            CallError::Fetch(FetchError::Auth("Dexcom did not accept this username and password, or the region is wrong.".into()))
        }
        "" => CallError::Fetch(FetchError::Other(format!("Dexcom answered with error {status}."))),
        c => CallError::Fetch(FetchError::Other(format!("Dexcom error: {c}"))),
    }
}

/// Parses `[{"WT":"Date(1691455258000)","Value":142,"Trend":"Flat"}, ...]`.
pub fn parse_readings(v: &Value) -> Result<Vec<Reading>, FetchError> {
    let arr = v
        .as_array()
        .ok_or_else(|| FetchError::Other("Unexpected answer from Dexcom.".into()))?;
    let mut out: Vec<Reading> = arr
        .iter()
        .filter_map(|e| {
            let mgdl = e.get("Value")?.as_u64()? as u16;
            let time_ms = parse_date(e.get("WT").or_else(|| e.get("ST"))?.as_str()?)?;
            let trend = match e.get("Trend") {
                Some(Value::String(s)) => Trend::from_name(s),
                Some(Value::Number(n)) => Trend::from_number(n.as_i64().unwrap_or(0)),
                _ => Trend::Unknown,
            };
            Some(Reading {
                mgdl,
                trend,
                time_ms,
            })
        })
        .collect();
    out.sort_by_key(|r| r.time_ms);
    Ok(out)
}

/// "Date(1691455258000)" or "Date(1691455258000-0400)" → 1691455258000.
fn parse_date(s: &str) -> Option<i64> {
    let inner = s.strip_prefix("Date(")?;
    let digits: String = inner.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic response in Dexcom's format. Not real patient data.
    const SAMPLE: &str = r#"[
        {"WT":"Date(1700000600000)","ST":"Date(1700000600000)","DT":"Date(1700000600000-0500)","Value":142,"Trend":"Flat"},
        {"WT":"Date(1700000300000)","ST":"Date(1700000300000)","DT":"Date(1700000300000-0500)","Value":140,"Trend":"FortyFiveUp"},
        {"WT":"Date(1700000000000)","ST":"Date(1700000000000)","DT":"Date(1700000000000-0500)","Value":131,"Trend":4}
    ]"#;

    #[test]
    fn parses_and_sorts() {
        let v: Value = serde_json::from_str(SAMPLE).unwrap();
        let r = parse_readings(&v).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].mgdl, 131);
        assert_eq!(r[0].trend, Trend::Flat);
        assert_eq!(r[2].mgdl, 142);
        assert_eq!(r[2].time_ms, 1_700_000_600_000);
    }

    #[test]
    fn password_errors_are_auth_errors() {
        let v = json!({"Code":"AccountPasswordInvalid","Message":"x"});
        assert!(matches!(
            classify_error(&v, 500),
            CallError::Fetch(FetchError::Auth(_))
        ));
        let v = json!({"Code":"SSO_AuthenticateMaxAttemptsExceeded"});
        assert!(matches!(
            classify_error(&v, 500),
            CallError::Fetch(FetchError::Auth(_))
        ));
        let v = json!({"Code":"SessionIdNotFound"});
        assert!(matches!(classify_error(&v, 500), CallError::SessionExpired));
    }

    #[test]
    fn dates() {
        assert_eq!(
            parse_date("Date(1691455258000-0400)"),
            Some(1_691_455_258_000)
        );
        assert_eq!(parse_date("nope"), None);
    }
}
