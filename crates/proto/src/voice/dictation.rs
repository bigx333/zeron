//! Codex dictation control. The execution host lends its ChatGPT access token;
//! the device with the microphone uploads audio straight to ChatGPT. Audio
//! never crosses RPC or the relay, and these payloads are never persisted.
use serde::{Deserialize, Serialize};
use std::fmt;

pub const CAPABILITY: &str = "codex-dictation-v1";
/// The only destination a client may send a lent credential to.
pub const TRANSCRIBE_URL: &str = "https://chatgpt.com/backend-api/transcribe";
/// `/transcribe` sits behind a bot check that only admits Codex's desktop app,
/// so uploads present its originator and User-Agent shape. Bump the version
/// with the Codex app if the check starts rejecting it.
pub const CODEX_DESKTOP_ORIGINATOR: &str = "Codex Desktop";
pub const CODEX_DESKTOP_VERSION: &str = "26.1007.71326";
/// Same bound as local dictation: a hold records at most one minute.
pub const MAX_SECONDS: u32 = 60;
/// A 60 s, 48 kHz, 16-bit mono WAV with headroom for container overhead.
pub const MAX_UPLOAD_BYTES: usize = 6 * 1024 * 1024;
const MAX_TOKEN_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DictationUnavailable {
    /// No saved or live Codex account signed in with ChatGPT on the host.
    NoChatgptAccount,
    /// The account exists but its credential could not be read or refreshed.
    SignInRequired,
    /// The host is an older build without Codex dictation.
    Unsupported,
    Protocol,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Status {
    pub available: bool,
    #[serde(default)]
    pub reason: Option<DictationUnavailable>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialRequest {
    /// Set after ChatGPT rejected the previous token.
    #[serde(default)]
    pub refresh: bool,
}

/// Unavailability is a reply, not an RPC error, so the reason survives the relay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum CredentialReply {
    Lent { credential: Credential },
    Unavailable { reason: DictationUnavailable },
}

/// A ChatGPT access token lent for one purpose. Never the refresh token.
/// Debug omits the secret; clients keep it in memory only.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Credential {
    access_token: String,
    #[serde(default)]
    pub account_id: Option<String>,
    /// Unix milliseconds from the token's own claim, when it has one.
    #[serde(default)]
    pub expires_at_ms: Option<i64>,
}
impl Credential {
    pub fn new(
        access_token: String,
        account_id: Option<String>,
        expires_at_ms: Option<i64>,
    ) -> Option<Self> {
        Self {
            access_token,
            account_id,
            expires_at_ms,
        }
        .validated()
    }
    pub fn expose(&self) -> &str {
        &self.access_token
    }
    /// Header-safe and bounded, so a hostile host cannot inject headers.
    pub fn validated(self) -> Option<Self> {
        let header_safe = |s: &str| s.bytes().all(|b| b.is_ascii_graphic());
        (!self.access_token.is_empty()
            && self.access_token.len() <= MAX_TOKEN_BYTES
            && header_safe(&self.access_token)
            && self
                .account_id
                .as_deref()
                .is_none_or(|id| !id.is_empty() && id.len() <= 256 && header_safe(id)))
        .then_some(self)
    }
    /// Expired, or expiring within `margin_ms`. Unknown expiry is trusted
    /// until ChatGPT rejects it.
    pub fn expires_within(&self, now_ms: i64, margin_ms: i64) -> bool {
        self.expires_at_ms
            .is_some_and(|at| at <= now_ms.saturating_add(margin_ms))
    }
}
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("account_id", &self.account_id)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish_non_exhaustive()
    }
}

/// `Codex Desktop/<version> (<platform>; <arch>)`, as Codex's app builds it.
pub fn codex_desktop_user_agent() -> String {
    let platform = match std::env::consts::OS {
        "linux" => "X11; Linux",
        "windows" => "Windows NT 10.0",
        _ => "Mac OS",
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    format!("Codex Desktop/{CODEX_DESKTOP_VERSION} ({platform}; {arch})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_debug_redacts_the_token() {
        let credential =
            Credential::new("secret-token".into(), Some("acct".into()), Some(1)).expect("valid");
        let debug = format!("{credential:?}");
        assert!(!debug.contains("secret-token"));
        assert!(debug.contains("acct"));
    }

    #[test]
    fn credential_rejects_header_injection() {
        assert!(Credential::new("a\r\nX-Evil: 1".into(), None, None).is_none());
        assert!(Credential::new("token".into(), Some("a b".into()), None).is_none());
        assert!(Credential::new(String::new(), None, None).is_none());
        let wire: Credential =
            serde_json::from_value(serde_json::json!({"accessToken": "t\n"})).unwrap();
        assert!(wire.validated().is_none());
    }

    #[test]
    fn expiry_margin() {
        let credential = Credential::new("t".into(), None, Some(10_000)).unwrap();
        assert!(!credential.expires_within(0, 5_000));
        assert!(credential.expires_within(6_000, 5_000));
        let unknown = Credential::new("t".into(), None, None).unwrap();
        assert!(!unknown.expires_within(i64::MAX - 1, 5_000));
    }
}
