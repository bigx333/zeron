//! Codex dictation over real engine RPC against the fake Codex app-server:
//! the host reports whether it can lend a ChatGPT credential and lends the
//! live login's token. Its own binary, since it points process env at fixtures.
#![cfg(unix)]

use base64::Engine as _;
use serde_json::json;
use zeron_engine::{EngineCore, EngineProfile, HarnessRegistry};
use zeron_proto::HarnessId;
use zeron_rpc::methods;

#[tokio::test]
async fn host_reports_and_lends_the_live_chatgpt_login() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../harness/tests/fixtures/fake-codex.sh");
    std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o755)).unwrap();
    let codex_home = temp.path().join("codex");
    std::fs::create_dir_all(&codex_home).unwrap();
    // SAFETY: this binary has a single test; nothing else reads the env concurrently.
    unsafe {
        std::env::set_var("CODEX_EXECUTABLE", &fixture);
        std::env::set_var("CODEX_HOME", &codex_home);
    }
    let core = EngineCore::assemble_with_profile(
        EngineProfile::local(&temp.path().join("data")).unwrap(),
        std::sync::Arc::new(HarnessRegistry::new()),
        HarnessId::Codex,
        None,
    )
    .unwrap();
    let client = zeron_rpc::memory_client(core.rpc_service());
    let envelope = |payload| json!({"targetDeviceId": core.device_id, "payload": payload});

    let status = client
        .call(methods::CODEX_DICTATION_STATUS_V1, envelope(json!({})))
        .await
        .unwrap();
    assert_eq!(
        status,
        json!({"available": false, "reason": "noChatgptAccount"})
    );

    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        json!({"email":"live@example.com","https://api.openai.com/auth":{"chatgpt_account_id":"acct-live"}})
            .to_string(),
    );
    std::fs::write(
        codex_home.join("auth.json"),
        json!({"tokens":{"id_token":format!("e30.{claims}.s"),"access_token":"t"}}).to_string(),
    )
    .unwrap();
    let status = client
        .call(methods::CODEX_DICTATION_STATUS_V1, envelope(json!({})))
        .await
        .unwrap();
    assert_eq!(status["available"], true);

    for (refresh, account) in [(false, "acct-live"), (true, "acct-refreshed")] {
        let reply = client
            .call(
                methods::CODEX_DICTATION_CREDENTIAL_V1,
                envelope(json!({"refresh": refresh})),
            )
            .await
            .unwrap();
        assert_eq!(reply["kind"], "lent", "{refresh}");
        assert_eq!(reply["credential"]["accountId"], account);
        assert_eq!(reply["credential"]["expiresAtMs"], 2_000_000_000_000i64);
        assert!(
            reply["credential"]["accessToken"]
                .as_str()
                .is_some_and(|t| t.starts_with("eyJ"))
        );
    }

    // Addressed to this host by envelope only; unknown fields are refused.
    assert!(
        client
            .call(
                methods::CODEX_DICTATION_CREDENTIAL_V1,
                envelope(json!({"refresh": false, "audio": "nope"})),
            )
            .await
            .is_err()
    );
}
