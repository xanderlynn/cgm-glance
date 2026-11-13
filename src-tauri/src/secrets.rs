//! Sign-in details, stored only in the OS keychain:
//! Windows Credential Manager, macOS Keychain, or Linux Secret Service.
//! There is deliberately no file fallback.

use serde::{Deserialize, Serialize};

const SERVICE: &str = "cgm-glance";

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Secret {
    DexcomShare { username: String, password: String },
    Nightscout { token: Option<String> },
}

// Never print secrets, even in debug logs.
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Secret::DexcomShare { .. } => f.write_str("Secret::DexcomShare(<redacted>)"),
            Secret::Nightscout { .. } => f.write_str("Secret::Nightscout(<redacted>)"),
        }
    }
}

fn entry(person_id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, person_id).map_err(explain)
}

fn explain(e: keyring::Error) -> String {
    match e {
        keyring::Error::NoEntry => "No saved sign-in was found in the system keychain.".into(),
        keyring::Error::NoStorageAccess(e) | keyring::Error::PlatformFailure(e) => {
            let hint = if cfg!(target_os = "macos") {
                "macOS didn't let CGM Glance read its saved sign-in. If a keychain prompt appeared, choose Always Allow; otherwise use \"Sign in again\"."
            } else if cfg!(windows) {
                "Windows Credential Manager couldn't be read. Use \"Sign in again\"."
            } else {
                "The system keychain is not available. Install and unlock GNOME Keyring or KWallet, then try again."
            };
            format!("{hint} ({e})")
        }
        other => format!("System keychain error: {other}"),
    }
}

pub fn store(person_id: &str, secret: &Secret) -> Result<(), String> {
    let json = serde_json::to_string(secret).map_err(|e| e.to_string())?;
    entry(person_id)?.set_password(&json).map_err(explain)
}

pub fn load(person_id: &str) -> Result<Secret, String> {
    let json = entry(person_id)?.get_password().map_err(explain)?;
    serde_json::from_str(&json)
        .map_err(|_| "Saved sign-in is unreadable. Sign in again.".to_string())
}

pub fn delete(person_id: &str) {
    if let Ok(e) = entry(person_id) {
        let _ = e.delete_credential();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_secret() {
        let s = Secret::DexcomShare {
            username: "alex@example.com".into(),
            password: "hunter2".into(),
        };
        let out = format!("{s:?}");
        assert!(!out.contains("hunter2") && !out.contains("alex@"));
    }
}
