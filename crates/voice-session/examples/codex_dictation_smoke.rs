//! Development check only: uploads one recording (WAV, or `.m4a` as phones
//! record) through the production Codex dictation client and prints the
//! transcript. One call uses real ChatGPT usage.
//!
//! Against a running engine, the whole production path (status, a credential
//! lent by the host's Codex app-server, upload from here):
//! `cargo run -p zeron-voice-session --example codex_dictation_smoke -- clip.wav ws://127.0.0.1:49777`
//!
//! Or with a token from `ZERON_DICTATION_TOKEN` (never printed), no engine:
//! `cargo run -p zeron-voice-session --example codex_dictation_smoke -- clip.wav`
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::{Value, json};
use std::sync::Arc;
use zeron_proto::voice::{VoiceEvent, VoiceRejection, remote as wire};
use zeron_voice_session::{
    RpcTransport, VoiceControlTransport,
    dictation::{Audio, CodexDictation},
};

struct EnvHost;
#[async_trait]
impl VoiceControlTransport for EnvHost {
    async fn call(&self, _: &str, _: Value, _: u64) -> Result<Value, VoiceRejection> {
        let token = std::env::var("ZERON_DICTATION_TOKEN").map_err(|_| VoiceRejection::Protocol)?;
        let account = std::env::var("ZERON_DICTATION_ACCOUNT").ok();
        Ok(json!({"kind":"lent","credential":{"accessToken":token,"accountId":account}}))
    }
    async fn own(
        &self,
        _: &wire::Lease,
    ) -> Result<BoxStream<'static, Result<VoiceEvent, VoiceRejection>>, VoiceRejection> {
        Err(VoiceRejection::Unsupported)
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: codex_dictation_smoke <clip> [engine ws url]");
    let control: Arc<dyn VoiceControlTransport> = match args.next() {
        Some(url) => {
            let client = zeron_rpc::connect_ws(&url).await.expect("engine reachable");
            let info = client
                .call(zeron_rpc::methods::ENGINE_INFO, json!({}))
                .await
                .expect("engine info");
            let host = info["deviceId"].as_str().expect("device id").to_owned();
            Arc::new(RpcTransport {
                client: Arc::new(client),
                host,
            })
        }
        None => Arc::new(EnvHost),
    };
    let dictation = CodexDictation::new(control);
    println!("host status: {:?}", dictation.status().await);
    let bytes = std::fs::read(&path).expect("readable recording");
    let (mime, filename) = if path.ends_with(".m4a") {
        ("audio/mp4", "dictation.m4a")
    } else {
        ("audio/wav", "dictation.wav")
    };
    let started = std::time::Instant::now();
    match dictation
        .transcribe(&Audio::encoded(bytes, mime, filename), None)
        .await
    {
        Ok(text) => println!("{text:?} in {:?}", started.elapsed()),
        Err(error) => println!("failed: {error:?}"),
    }
}
