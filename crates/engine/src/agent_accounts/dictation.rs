//! Codex dictation lends a ChatGPT access token to the owner's devices, which
//! upload audio straight to ChatGPT. The live login is read through Codex's
//! app-server by the RPC layer; this is the fallback over saved accounts.
//! Pool members are saved slots, so one scan covers native and pooled accounts.
use zeron_harness::codex::ChatgptToken;
use zeron_proto::voice::dictation::{Credential, DictationUnavailable, Status};

use super::*;

impl AgentAccounts {
    /// Any Codex account signed in with ChatGPT, live or saved. No network.
    pub fn codex_dictation_status(&self) -> Status {
        let available = self.has_live_codex_chatgpt_login()
            || self.read_slots(HarnessId::Codex).iter().any(chatgpt_slot);
        Status {
            available,
            reason: (!available).then_some(DictationUnavailable::NoChatgptAccount),
        }
    }

    pub fn has_live_codex_chatgpt_login(&self) -> bool {
        self.detect_codex()
            .is_some_and(|detected| detected.profile.auth_kind == AgentAuthKind::Oauth)
    }

    /// A saved ChatGPT account's token, refreshed by the pool's own machinery.
    /// With the Codex pool on, its order wins and accounts left out of it go
    /// last; otherwise the oldest saved account goes first.
    pub async fn codex_dictation_slot_credential(
        &self,
        refresh: bool,
    ) -> Result<Credential, DictationUnavailable> {
        let mut slots: Vec<Slot> = self
            .read_slots(HarnessId::Codex)
            .into_iter()
            .filter(chatgpt_slot)
            .collect();
        if slots.is_empty() {
            return Err(DictationUnavailable::NoChatgptAccount);
        }
        let pool = self.enabled_pool(HarnessId::Codex);
        slots.sort_by_key(|slot| {
            let (excluded, position) = pool.as_ref().map_or((false, 0), |pool| {
                (
                    pool.disabled_account_ids.contains(&slot.id),
                    pool.account_order
                        .iter()
                        .position(|id| id == &slot.id)
                        .unwrap_or(usize::MAX),
                )
            });
            (
                excluded,
                position,
                slot.created_at.unwrap_or(slot.saved_at),
                slot.id.clone(),
            )
        });
        for slot in slots {
            match self.pool_secret(&slot, refresh).await {
                Ok(current) => {
                    if let Some(credential) = slot_credential(&current) {
                        return Ok(credential);
                    }
                }
                Err(error) => tracing::debug!(
                    slot = %slot.id,
                    reason = error.message(),
                    "codex dictation: saved account cannot lend a token"
                ),
            }
        }
        Err(DictationUnavailable::SignInRequired)
    }
}

fn chatgpt_slot(slot: &Slot) -> bool {
    slot.profile.auth_kind == AgentAuthKind::Oauth
        && str_field(&slot.credentials["tokens"], "access_token").is_some()
}

fn slot_credential(slot: &Slot) -> Option<Credential> {
    let tokens = &slot.credentials["tokens"];
    let token = ChatgptToken::from_jwt(str_field(tokens, "access_token")?);
    let account_id = str_field(tokens, "account_id").or(token.account_id);
    Credential::new(token.access_token, account_id, token.expires_at_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accounts() -> (tempfile::TempDir, AgentAccounts) {
        let root = tempfile::tempdir().unwrap();
        let accounts = AgentAccounts::with_endpoints(
            AgentAccountsConfig::isolated(root.path()),
            ProbeEndpoints {
                codex_usage: "http://127.0.0.1:9/usage".into(),
                ..Default::default()
            },
            Default::default(),
        );
        (root, accounts)
    }

    fn save(accounts: &AgentAccounts, name: &str, created_at: i64, kind: AgentAuthKind) -> Slot {
        let slot = Slot {
            id: slot_id_for(HarnessId::Codex, name),
            harness: HarnessId::Codex,
            account_key: name.into(),
            profile: SlotProfile {
                email: format!("{name}@example.com"),
                display_name: None,
                organization: None,
                plan: None,
                auth_kind: kind,
            },
            credentials: match kind {
                AgentAuthKind::Oauth => serde_json::json!({"tokens":{
                    "access_token":format!("token-{name}"),
                    "refresh_token":format!("refresh-{name}"),
                    "account_id":format!("acct-{name}"),
                }}),
                AgentAuthKind::ApiKey => serde_json::json!({"OPENAI_API_KEY":"sk-test"}),
            },
            claude_config: None,
            saved_at: now_ms(),
            created_at: Some(created_at),
            store_key: None,
        };
        accounts.write_slot(&slot).unwrap();
        slot
    }

    #[tokio::test]
    async fn api_keys_and_empty_stores_cannot_lend_a_chatgpt_token() {
        let (_root, accounts) = accounts();
        assert_eq!(
            accounts.codex_dictation_status().reason,
            Some(DictationUnavailable::NoChatgptAccount)
        );
        save(&accounts, "key", 1, AgentAuthKind::ApiKey);
        assert!(!accounts.codex_dictation_status().available);
        assert_eq!(
            accounts.codex_dictation_slot_credential(false).await,
            Err(DictationUnavailable::NoChatgptAccount)
        );
    }

    #[tokio::test]
    async fn saved_accounts_lend_oldest_first_and_follow_the_pool_order() {
        let (_root, accounts) = accounts();
        let older = save(&accounts, "older", 1, AgentAuthKind::Oauth);
        let newer = save(&accounts, "newer", 2, AgentAuthKind::Oauth);
        assert!(accounts.codex_dictation_status().available);
        let credential = accounts
            .codex_dictation_slot_credential(false)
            .await
            .unwrap();
        assert_eq!(credential.expose(), "token-older");
        assert_eq!(credential.account_id.as_deref(), Some("acct-older"));

        accounts.edit_pool_config(|config| {
            config.codex.enabled = true;
            config.codex.account_order = vec![newer.id.clone(), older.id.clone()];
        });
        let credential = accounts
            .codex_dictation_slot_credential(false)
            .await
            .unwrap();
        assert_eq!(credential.expose(), "token-newer");

        // Left out of the pool, still a configured account: used last.
        accounts.edit_pool_config(|config| {
            config.codex.disabled_account_ids.push(newer.id.clone());
        });
        let credential = accounts
            .codex_dictation_slot_credential(false)
            .await
            .unwrap();
        assert_eq!(credential.expose(), "token-older");
    }

    #[test]
    fn a_live_chatgpt_login_counts_as_configured() {
        let (root, accounts) = accounts();
        let claims = BASE64_URL.encode(
            serde_json::json!({
                "email":"live@example.com",
                "https://api.openai.com/auth":{"chatgpt_account_id":"acct-live"},
            })
            .to_string(),
        );
        let codex = root.path().join("codex");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(
            codex.join("auth.json"),
            serde_json::json!({"tokens":{"id_token":format!("e30.{claims}.s"),"access_token":"t"}})
                .to_string(),
        )
        .unwrap();
        assert!(accounts.has_live_codex_chatgpt_login());
        assert!(accounts.codex_dictation_status().available);
    }
}
