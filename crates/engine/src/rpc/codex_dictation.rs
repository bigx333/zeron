//! Codex dictation, realtime-style: the host owns the Codex login, the device
//! with the microphone owns the audio. Only the lent credential crosses here.
use super::*;
use zeron_proto::voice::dictation::{
    Credential, CredentialReply, CredentialRequest, DictationUnavailable,
};
use zeron_proto::voice::remote::Envelope;

pub(super) fn handles(method: &str) -> bool {
    matches!(
        method,
        methods::CODEX_DICTATION_STATUS_V1 | methods::CODEX_DICTATION_CREDENTIAL_V1
    )
}
fn failure(reason: DictationUnavailable) -> RpcError {
    RpcError::Failed(format!("codex dictation unavailable: {reason:?}"))
}

impl EngineRpc {
    pub(super) async fn codex_dictation(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<RpcReply, RpcError> {
        let envelope: Envelope<serde_json::Value> = parse_params(params)?;
        if envelope.target_device_id != self.engine_info.device_id {
            return Err(failure(DictationUnavailable::Protocol));
        }
        match method {
            methods::CODEX_DICTATION_STATUS_V1 => {
                RpcReply::value(&self.agent_accounts.codex_dictation_status())
            }
            methods::CODEX_DICTATION_CREDENTIAL_V1 => {
                let request: CredentialRequest = parse_params(envelope.payload)?;
                RpcReply::value(
                    &match self.codex_dictation_credential(request.refresh).await {
                        Ok(credential) => CredentialReply::Lent { credential },
                        Err(reason) => CredentialReply::Unavailable { reason },
                    },
                )
            }
            _ => Err(RpcError::UnknownMethod(method.to_string())),
        }
    }

    /// The live login through Codex's app-server, the way Codex's desktop app
    /// reads it; Codex owns that refresh. Without one, a saved or pooled account.
    async fn codex_dictation_credential(
        &self,
        refresh: bool,
    ) -> Result<Credential, DictationUnavailable> {
        match zeron_harness::CodexHarness::new()
            .chatgpt_access_token(refresh)
            .await
        {
            Ok(Some(token)) => {
                if let Some(credential) =
                    Credential::new(token.access_token, token.account_id, token.expires_at_ms)
                {
                    return Ok(credential);
                }
            }
            Ok(None) => {}
            Err(error) => tracing::debug!(
                %error,
                "codex dictation: app-server cannot lend the live login; trying saved accounts"
            ),
        }
        self.agent_accounts
            .codex_dictation_slot_credential(refresh)
            .await
    }
}
