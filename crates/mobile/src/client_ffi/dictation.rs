//! Codex dictation for phones, the realtime pattern: the platform records,
//! the host device lends its ChatGPT credential over the relay, and the upload
//! goes from this phone straight to ChatGPT. Audio never crosses the relay,
//! and the credential never crosses UniFFI.
use super::*;
use std::collections::HashMap;
use std::sync::Mutex;
use zeron_proto::voice::dictation::DictationUnavailable;
use zeron_voice_session::dictation::{Audio, CodexDictation, DictationError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CodexDictationUnavailable {
    /// No Codex account signed in with ChatGPT on the host.
    NoChatgptAccount,
    /// The account must sign in again on the host.
    SignInRequired,
    /// The host runs a build without Codex dictation.
    HostUpdateRequired,
    HostUnreachable,
}
impl From<DictationUnavailable> for CodexDictationUnavailable {
    fn from(reason: DictationUnavailable) -> Self {
        match reason {
            DictationUnavailable::NoChatgptAccount => Self::NoChatgptAccount,
            DictationUnavailable::SignInRequired => Self::SignInRequired,
            DictationUnavailable::Unsupported => Self::HostUpdateRequired,
            DictationUnavailable::Protocol => Self::HostUnreachable,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CodexDictationStatus {
    pub available: bool,
    pub reason: Option<CodexDictationUnavailable>,
    /// Ready to show when unavailable.
    pub message: Option<String>,
}

/// What the platform recorder produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum DictationAudioFormat {
    /// AAC in an MPEG-4 container: AVAudioRecorder / MediaRecorder.
    M4a,
    Wav,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum CodexDictationError {
    /// `reason` is user-facing; the draft is never touched. (Not `message`:
    /// Kotlin exceptions already have one, and the bindings would not compile.)
    #[error("{reason}")]
    Failed { reason: String },
}

/// Per host: relay links are swapped on reconnect, the lent credential is kept.
#[derive(Default)]
pub(crate) struct DictationClients(Mutex<HashMap<String, Arc<CodexDictation>>>);

impl CoreClient {
    async fn dictation(&self, host: &str) -> Result<Arc<CodexDictation>, DictationError> {
        let client = self.client.clone();
        let device = host.to_owned();
        let control = zc::runtime::run(async move { client.voice_transport(&device).await })
            .await
            .map_err(|_| DictationError::Unavailable(DictationUnavailable::Protocol))?;
        let mut clients = self.dictation.0.lock().unwrap_or_else(|e| e.into_inner());
        let dictation = clients
            .entry(host.to_owned())
            .or_insert_with(|| Arc::new(CodexDictation::new(control.clone())))
            .clone();
        dictation.set_control(control);
        Ok(dictation)
    }
}

#[uniffi::export]
impl CoreClient {
    /// Whether `host_device_id` can lend a ChatGPT credential (a Codex
    /// account signed in with ChatGPT, live or saved/pooled). No upload.
    pub async fn codex_dictation_status(&self, host_device_id: String) -> CodexDictationStatus {
        let status = match self.dictation(&host_device_id).await {
            Ok(dictation) => zc::runtime::run(async move { Ok(dictation.status().await) })
                .await
                .unwrap_or(Err(DictationUnavailable::Protocol)),
            Err(_) => Err(DictationUnavailable::Protocol),
        };
        let reason = match status {
            Ok(status) if status.available => None,
            Ok(status) => Some(
                status
                    .reason
                    .unwrap_or(DictationUnavailable::NoChatgptAccount),
            ),
            Err(reason) => Some(reason),
        };
        CodexDictationStatus {
            available: reason.is_none(),
            reason: reason.map(Into::into),
            message: reason.map(|r| DictationError::Unavailable(r).message().to_owned()),
        }
    }

    /// Call when recording starts: fetches the credential while the user talks.
    pub async fn prepare_codex_dictation(&self, host_device_id: String) {
        if let Ok(dictation) = self.dictation(&host_device_id).await {
            let _ = zc::runtime::run(async move { Ok(dictation.prefetch().await) }).await;
        }
    }

    /// Uploads a finished recording to ChatGPT; `""` when no speech was heard.
    pub async fn transcribe_codex_dictation(
        &self,
        host_device_id: String,
        audio: Vec<u8>,
        format: DictationAudioFormat,
    ) -> Result<String, CodexDictationError> {
        let failed = |error: DictationError| CodexDictationError::Failed {
            reason: error.message().to_owned(),
        };
        let dictation = self.dictation(&host_device_id).await.map_err(failed)?;
        let audio = match format {
            DictationAudioFormat::M4a => Audio::encoded(audio, "audio/mp4", "dictation.m4a"),
            DictationAudioFormat::Wav => Audio::encoded(audio, "audio/wav", "dictation.wav"),
        };
        zc::runtime::run(async move { Ok(dictation.transcribe(&audio, None).await) })
            .await
            .unwrap_or(Err(DictationError::Network))
            .map_err(failed)
    }
}

/// Hosts advertising this capability lend credentials for Codex dictation.
#[uniffi::export]
pub fn codex_dictation_host_capability() -> String {
    zeron_proto::voice::dictation::CAPABILITY.into()
}
