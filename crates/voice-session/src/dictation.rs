//! Codex dictation, shared by desktop and mobile. Like a call, the host owns
//! Codex and the platform owns media: the host lends its ChatGPT access token
//! over RPC and this device uploads the recording straight to ChatGPT, so
//! audio never crosses RPC or the relay.
use crate::VoiceControlTransport;
use serde_json::json;
use std::time::{Duration, Instant};
use zeron_proto::voice::VoiceRejection;
use zeron_proto::voice::dictation::{
    CODEX_DESKTOP_ORIGINATOR, Credential, CredentialReply, CredentialRequest, DictationUnavailable,
    MAX_UPLOAD_BYTES, Status, TRANSCRIBE_URL, codex_desktop_user_agent,
};
use zeron_rpc::methods;

/// Bounds how long a lent token outlives a sign-out or account switch on the host.
const CREDENTIAL_TTL: Duration = Duration::from_secs(10 * 60);
const EXPIRY_MARGIN_MS: i64 = 60_000;
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DictationError {
    Unavailable(DictationUnavailable),
    /// ChatGPT kept rejecting the host's token, even after a refresh.
    Unauthorized,
    RateLimited,
    TooLarge,
    Network,
    Service(u16),
    InvalidResponse,
}
impl DictationError {
    /// Short, user-facing; the draft is never touched on failure.
    pub fn message(&self) -> &'static str {
        match self {
            Self::Unavailable(DictationUnavailable::NoChatgptAccount) => {
                "Sign in to Codex with ChatGPT on the host device to use Codex dictation."
            }
            Self::Unavailable(DictationUnavailable::SignInRequired) | Self::Unauthorized => {
                "Codex needs you to sign in again on the host device. Your draft is safe."
            }
            Self::Unavailable(DictationUnavailable::Unsupported) => {
                "Update Zeron on the host device to use Codex dictation."
            }
            Self::Unavailable(DictationUnavailable::Protocol) => {
                "Couldn’t reach the host device. Your draft is safe."
            }
            Self::RateLimited => "Codex dictation is busy. Wait a moment, then try again.",
            Self::TooLarge => "That recording is too long to transcribe.",
            Self::Network => "Couldn’t reach ChatGPT. Check your connection. Your draft is safe.",
            Self::Service(_) | Self::InvalidResponse => {
                "ChatGPT couldn’t transcribe this recording. Your draft is safe."
            }
        }
    }
}

/// An encoded recording. No Debug: audio must never be logged implicitly.
pub struct Audio {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub filename: &'static str,
}
impl Audio {
    /// 16-bit PCM mono WAV from float samples in −1…1.
    pub fn wav(samples: &[f32], rate: u32) -> Self {
        let data = (samples.len() * 2) as u32;
        let mut bytes = Vec::with_capacity(44 + data as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        for sample in samples {
            let pcm = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            bytes.extend_from_slice(&pcm.to_le_bytes());
        }
        Self {
            bytes,
            mime: "audio/wav",
            filename: "dictation.wav",
        }
    }
    /// An already encoded file from a platform recorder.
    pub fn encoded(bytes: Vec<u8>, mime: &'static str, filename: &'static str) -> Self {
        Self {
            bytes,
            mime,
            filename,
        }
    }
}

pub async fn status(control: &dyn VoiceControlTransport) -> Result<Status, DictationUnavailable> {
    let value = control
        .call(methods::CODEX_DICTATION_STATUS_V1, json!({}), 8)
        .await
        .map_err(unavailable)?;
    serde_json::from_value(value).map_err(|_| DictationUnavailable::Protocol)
}

/// One per host. Caches the lent credential in memory only.
pub struct CodexDictation {
    control: std::sync::Mutex<std::sync::Arc<dyn VoiceControlTransport>>,
    http: reqwest::Client,
    endpoint: String,
    credential: tokio::sync::Mutex<Option<(Credential, Instant)>>,
}
impl CodexDictation {
    pub fn new(control: std::sync::Arc<dyn VoiceControlTransport>) -> Self {
        Self::with_endpoint(control, TRANSCRIBE_URL.into())
    }
    fn with_endpoint(control: std::sync::Arc<dyn VoiceControlTransport>, endpoint: String) -> Self {
        Self {
            control: std::sync::Mutex::new(control),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
            endpoint,
            credential: Default::default(),
        }
    }

    /// Relay links are replaced on reconnect; keep the cached credential.
    pub fn set_control(&self, control: std::sync::Arc<dyn VoiceControlTransport>) {
        *self.control.lock().unwrap_or_else(|e| e.into_inner()) = control;
    }
    fn control(&self) -> std::sync::Arc<dyn VoiceControlTransport> {
        self.control
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub async fn status(&self) -> Result<Status, DictationUnavailable> {
        status(self.control().as_ref()).await
    }

    /// Fetch the credential while the user is still talking.
    pub async fn prefetch(&self) -> Result<(), DictationError> {
        self.credential(false).await.map(drop)
    }

    async fn credential(&self, refresh: bool) -> Result<Credential, DictationError> {
        let mut cached = self.credential.lock().await;
        if !refresh
            && let Some((credential, fetched)) = cached.as_ref()
            && fetched.elapsed() < CREDENTIAL_TTL
            && !credential.expires_within(now_ms(), EXPIRY_MARGIN_MS)
        {
            return Ok(credential.clone());
        }
        *cached = None;
        let value = self
            .control()
            .call(
                methods::CODEX_DICTATION_CREDENTIAL_V1,
                serde_json::to_value(CredentialRequest { refresh })
                    .map_err(|_| DictationError::InvalidResponse)?,
                50,
            )
            .await
            .map_err(|reason| DictationError::Unavailable(unavailable(reason)))?;
        let credential = match serde_json::from_value::<CredentialReply>(value) {
            Ok(CredentialReply::Lent { credential }) => credential
                .validated()
                .ok_or(DictationError::Unavailable(DictationUnavailable::Protocol))?,
            Ok(CredentialReply::Unavailable { reason }) => {
                return Err(DictationError::Unavailable(reason));
            }
            Err(_) => return Err(DictationError::Unavailable(DictationUnavailable::Protocol)),
        };
        *cached = Some((credential.clone(), Instant::now()));
        Ok(credential)
    }

    /// Final text, or empty when ChatGPT heard no speech.
    pub async fn transcribe(
        &self,
        audio: &Audio,
        language: Option<&str>,
    ) -> Result<String, DictationError> {
        if audio.bytes.len() > MAX_UPLOAD_BYTES {
            return Err(DictationError::TooLarge);
        }
        let boundary = format!("zeron-{}", uuid::Uuid::new_v4().simple());
        let body = multipart(&boundary, audio, language);
        let mut refreshed = false;
        let mut attempt = 0;
        loop {
            let credential = self.credential(false).await?;
            let mut request = self
                .http
                .post(&self.endpoint)
                .timeout(UPLOAD_TIMEOUT)
                .bearer_auth(credential.expose())
                .header(
                    reqwest::header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header(reqwest::header::USER_AGENT, codex_desktop_user_agent())
                .header("originator", CODEX_DESKTOP_ORIGINATOR)
                .body(body.clone());
            if let Some(account) = &credential.account_id {
                request = request.header("ChatGPT-Account-Id", account);
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(_) if attempt < 2 => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(500 << attempt)).await;
                    continue;
                }
                Err(_) => return Err(DictationError::Network),
            };
            let status = response.status().as_u16();
            match status {
                200..=299 => {
                    let reply: serde_json::Value = response
                        .json()
                        .await
                        .map_err(|_| DictationError::InvalidResponse)?;
                    return reply["text"]
                        .as_str()
                        .map(|text| text.trim().to_owned())
                        .ok_or(DictationError::InvalidResponse);
                }
                401 | 403 if !refreshed => {
                    refreshed = true;
                    self.credential(true).await?;
                }
                401 | 403 => return Err(DictationError::Unauthorized),
                400 => {
                    let reply = response.text().await.unwrap_or_default();
                    // Too little audio is "no speech", not a failure.
                    return if reply.contains("audio_too_short") {
                        Ok(String::new())
                    } else {
                        Err(DictationError::Service(status))
                    };
                }
                413 => return Err(DictationError::TooLarge),
                429 => return Err(DictationError::RateLimited),
                500..=599 if attempt < 2 => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(500 << attempt)).await;
                }
                _ => return Err(DictationError::Service(status)),
            }
        }
    }
}

/// An older host answers the method as unknown.
fn unavailable(reason: VoiceRejection) -> DictationUnavailable {
    match reason {
        VoiceRejection::Unsupported => DictationUnavailable::Unsupported,
        _ => DictationUnavailable::Protocol,
    }
}

fn multipart(boundary: &str, audio: &Audio, language: Option<&str>) -> Vec<u8> {
    let mut body = Vec::with_capacity(audio.bytes.len() + 512);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
            audio.filename, audio.mime
        )
        .as_bytes(),
    );
    body.extend_from_slice(&audio.bytes);
    body.extend_from_slice(b"\r\n");
    if let Some(language) = language.filter(|l| {
        !l.is_empty() && l.len() <= 16 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }) {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\n{language}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream::BoxStream;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use zeron_proto::voice::{VoiceEvent, remote as wire};

    #[derive(Default)]
    struct Host {
        refreshes: Mutex<Vec<bool>>,
        reply: Mutex<Option<serde_json::Value>>,
    }
    #[async_trait]
    impl VoiceControlTransport for Host {
        async fn call(
            &self,
            method: &str,
            payload: serde_json::Value,
            _: u64,
        ) -> Result<serde_json::Value, VoiceRejection> {
            match method {
                methods::CODEX_DICTATION_CREDENTIAL_V1 => {
                    let refresh = payload["refresh"].as_bool().unwrap();
                    self.refreshes.lock().unwrap().push(refresh);
                    if let Some(reply) = self.reply.lock().unwrap().clone() {
                        return Ok(reply);
                    }
                    let token = if refresh { "fresh" } else { "stale" };
                    Ok(json!({"kind":"lent","credential":{"accessToken":token,"accountId":"acct"}}))
                }
                methods::CODEX_DICTATION_STATUS_V1 => Ok(json!({"available":true})),
                _ => Err(VoiceRejection::Unsupported),
            }
        }
        async fn own(
            &self,
            _: &wire::Lease,
        ) -> Result<BoxStream<'static, Result<VoiceEvent, VoiceRejection>>, VoiceRejection>
        {
            Err(VoiceRejection::Unsupported)
        }
    }

    struct Seen {
        authorization: String,
        account: String,
        originator: String,
        agent: String,
        body: Vec<u8>,
    }

    /// Answers each request with the next scripted (status, body) and records it.
    async fn chatgpt(script: Vec<(u16, &'static str)>) -> (String, Arc<Mutex<Vec<Seen>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/backend-api/transcribe",
            listener.local_addr().unwrap()
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            for (status, reply) in script {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut raw = Vec::new();
                let mut buf = [0u8; 8192];
                let (head, length) = loop {
                    let n = socket.read(&mut buf).await.unwrap();
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&raw[..end]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        raw.drain(..end + 4);
                        break (head, length);
                    }
                };
                while raw.len() < length {
                    let n = socket.read(&mut buf).await.unwrap();
                    raw.extend_from_slice(&buf[..n]);
                }
                let header = |name: &str| {
                    head.lines()
                        .find_map(|l| l.strip_prefix(name))
                        .unwrap_or_default()
                        .trim()
                        .to_owned()
                };
                log.lock().unwrap().push(Seen {
                    authorization: header("authorization: "),
                    account: header("chatgpt-account-id: "),
                    originator: header("originator: "),
                    agent: header("user-agent: "),
                    body: raw.clone(),
                });
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (url, seen)
    }

    fn dictation(host: Arc<Host>, url: String) -> CodexDictation {
        CodexDictation::with_endpoint(host, url)
    }

    #[tokio::test]
    async fn uploads_wav_with_the_lent_credential_and_reuses_it() {
        let (url, seen) = chatgpt(vec![
            (200, r#"{"text":" hello "}"#),
            (200, r#"{"text":"again"}"#),
        ])
        .await;
        let host = Arc::new(Host::default());
        let dictation = dictation(host.clone(), url);
        let audio = Audio::wav(&[0.0, 0.5, -1.0], 16_000);
        assert_eq!(&audio.bytes[..4], b"RIFF");
        assert_eq!(audio.bytes.len(), 44 + 6);
        assert_eq!(
            dictation.transcribe(&audio, Some("en")).await.unwrap(),
            "hello"
        );
        assert_eq!(dictation.transcribe(&audio, None).await.unwrap(), "again");
        assert_eq!(*host.refreshes.lock().unwrap(), vec![false]);
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].authorization, "bearer stale");
        assert_eq!(seen[0].account, "acct");
        assert_eq!(seen[0].originator, "codex desktop");
        assert!(seen[0].agent.starts_with("codex desktop/"));
        let body = String::from_utf8_lossy(&seen[0].body);
        assert!(body.contains("name=\"file\"; filename=\"dictation.wav\""));
        assert!(body.contains("Content-Type: audio/wav"));
        assert!(body.contains("name=\"language\"\r\n\r\nen\r\n"));
        assert!(!String::from_utf8_lossy(&seen[1].body).contains("name=\"language\""));
    }

    #[tokio::test]
    async fn a_rejected_token_is_refreshed_once() {
        let (url, seen) = chatgpt(vec![(401, "{}"), (200, r#"{"text":"ok"}"#)]).await;
        let host = Arc::new(Host::default());
        let dictation = dictation(host.clone(), url);
        let audio = Audio::wav(&[0.1; 160], 16_000);
        assert_eq!(dictation.transcribe(&audio, None).await.unwrap(), "ok");
        assert_eq!(*host.refreshes.lock().unwrap(), vec![false, true]);
        assert_eq!(seen.lock().unwrap()[1].authorization, "bearer fresh");

        let (url, _) = chatgpt(vec![(401, "{}"), (403, "{}")]).await;
        let dictation = super::CodexDictation::with_endpoint(Arc::new(Host::default()), url);
        assert_eq!(
            dictation.transcribe(&audio, None).await,
            Err(DictationError::Unauthorized)
        );
    }

    #[tokio::test]
    async fn too_short_is_no_speech_and_unavailable_is_reported() {
        let (url, _) = chatgpt(vec![(400, r#"{"detail":{"code":"audio_too_short"}}"#)]).await;
        let dictation = dictation(Arc::new(Host::default()), url);
        let audio = Audio::wav(&[0.0; 16], 16_000);
        assert_eq!(dictation.transcribe(&audio, None).await.unwrap(), "");

        let host = Arc::new(Host::default());
        *host.reply.lock().unwrap() =
            Some(json!({"kind":"unavailable","reason":"noChatgptAccount"}));
        let dictation = super::CodexDictation::with_endpoint(host, "http://127.0.0.1:9/".into());
        assert_eq!(
            dictation.transcribe(&audio, None).await,
            Err(DictationError::Unavailable(
                DictationUnavailable::NoChatgptAccount
            ))
        );
        assert!(dictation.status().await.unwrap().available);
    }

    #[tokio::test]
    async fn hostile_credentials_are_refused() {
        let host = Arc::new(Host::default());
        *host.reply.lock().unwrap() =
            Some(json!({"kind":"lent","credential":{"accessToken":"t\r\nX-Evil: 1"}}));
        let dictation = super::CodexDictation::with_endpoint(host, "http://127.0.0.1:9/".into());
        assert_eq!(
            dictation.prefetch().await,
            Err(DictationError::Unavailable(DictationUnavailable::Protocol))
        );
    }
}
