//! Codex dictation, the realtime pattern: capture stays on this desktop, the
//! Codex device (Settings → Voice) lends its ChatGPT credential, and this
//! device uploads the recording straight to ChatGPT. Audio never crosses RPC.
use crate::state::EngineHandle;
use gpui::{App, Global, Task};
use gpui_tokio::Tokio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use zeron_proto::voice::dictation::DictationUnavailable;
use zeron_voice_session::dictation::{Audio, CodexDictation};

/// Accounts change rarely, and a remote check rides the relay: re-read at most
/// this often unless forced (window activation, Settings → Voice).
const RECHECK: Duration = Duration::from_secs(5 * 60);

#[derive(Default)]
struct CodexGlobal {
    host: Option<String>,
    engine: Option<EngineHandle>,
    client: Option<Arc<CodexDictation>>,
    available: Option<bool>,
    reason: Option<DictationUnavailable>,
    checked: Option<Instant>,
    check: Option<Task<()>>,
}
impl Global for CodexGlobal {}

/// The Codex device last reported a ChatGPT account it can lend.
pub(crate) fn available(cx: &App) -> bool {
    cx.try_global::<CodexGlobal>()
        .is_some_and(|g| g.available == Some(true))
}

/// `None` while unknown (never checked, or checking).
pub(crate) fn status(cx: &App) -> Option<Result<(), DictationUnavailable>> {
    let global = cx.try_global::<CodexGlobal>()?;
    match global.available? {
        true => Some(Ok(())),
        false => Some(Err(global
            .reason
            .unwrap_or(DictationUnavailable::NoChatgptAccount))),
    }
}

/// Re-read whether `host` can lend a credential; no network on the host.
pub(crate) fn refresh(engine: &EngineHandle, host: String, force: bool, cx: &mut App) {
    let global = cx.default_global::<CodexGlobal>();
    // A host that stopped answering gets a fresh connection on the next check.
    let same = global.host.as_deref() == Some(host.as_str())
        && global
            .engine
            .as_ref()
            .is_some_and(|e| e.same_connection(engine))
        && global.reason != Some(DictationUnavailable::Protocol);
    if same
        && !force
        && (global.check.is_some() || global.checked.is_some_and(|at| at.elapsed() < RECHECK))
    {
        return;
    }
    if !same {
        *global = CodexGlobal {
            host: Some(host.clone()),
            engine: Some(engine.clone()),
            ..Default::default()
        };
    }
    let existing = global.client.clone();
    let engine = engine.clone();
    // Release the global: spawning borrows the app again.
    let query = Tokio::spawn(cx, async move {
        let client = match existing {
            Some(client) => client,
            None => {
                let rpc = Arc::new(engine.media_client().await.ok()?);
                let control = zeron_voice_session::RpcTransport { client: rpc, host };
                Arc::new(CodexDictation::new(Arc::new(control)))
            }
        };
        let status = client.status().await;
        Some((client, status))
    });
    let check = cx.spawn(async move |cx| {
        let result = query.await.ok().flatten();
        cx.update(|cx| {
            let global = cx.default_global::<CodexGlobal>();
            global.check = None;
            global.checked = Some(Instant::now());
            let (available, reason) = match result {
                Some((client, status)) => {
                    global.client = Some(client);
                    match status {
                        Ok(status) => (status.available, status.reason),
                        Err(reason) => (false, Some(reason)),
                    }
                }
                None => (false, Some(DictationUnavailable::Protocol)),
            };
            if global.available != Some(available) || global.reason != reason {
                tracing::debug!(
                    target: "zeron_ui::dictation",
                    available,
                    ?reason,
                    "Codex dictation availability"
                );
                global.available = Some(available);
                global.reason = reason;
                cx.refresh_windows();
            }
        });
    });
    cx.default_global::<CodexGlobal>().check = Some(check);
}

/// A worker-side transcriber for one recording, plus a credential prefetch
/// so the upload starts the moment the microphone closes.
pub(crate) fn backend(cx: &App) -> Option<zeron_voice::Backend> {
    let client = cx.try_global::<CodexGlobal>()?.client.clone()?;
    let runtime = Tokio::handle(cx);
    let prefetch = client.clone();
    runtime.spawn(async move {
        let _ = prefetch.prefetch().await;
    });
    Some(zeron_voice::Backend::Custom(Box::new(
        move |samples, rate| {
            let Some(speech) = zeron_voice::speech_16k(samples, rate)? else {
                return Ok(String::new());
            };
            let audio = Audio::wav(&speech, zeron_voice::SPEECH_RATE);
            runtime
                .block_on(client.transcribe(&audio, None))
                .map_err(|error| anyhow::anyhow!(error.message()))
        },
    )))
}

pub(crate) fn unavailable_message(reason: DictationUnavailable) -> &'static str {
    zeron_voice_session::dictation::DictationError::Unavailable(reason).message()
}

#[cfg(test)]
pub(crate) fn set_status_for_test(status: Option<Result<(), DictationUnavailable>>, cx: &mut App) {
    let global = cx.default_global::<CodexGlobal>();
    global.available = status.map(|s| s.is_ok());
    global.reason = status.and_then(Result::err);
}
