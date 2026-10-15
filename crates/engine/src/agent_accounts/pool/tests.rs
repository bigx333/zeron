use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::StreamExt;
use http_body_util::{BodyExt, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::{
    Request, Response,
    body::{Bytes, Frame, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;

use super::*;

type TestBody = UnsyncBoxBody<Bytes, std::io::Error>;
fn reply(status: u16, body: &'static str) -> Response<TestBody> {
    Response::builder()
        .status(status)
        .body(
            Full::new(Bytes::from_static(body.as_bytes()))
                .map_err(|never| match never {})
                .boxed_unsync(),
        )
        .unwrap()
}

struct Upstream {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn upstream<F>(handler: F) -> Upstream
where
    F: Fn(reqwest::header::HeaderMap, Bytes) -> Response<TestBody> + Send + Sync + 'static,
{
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handler = Arc::new(handler);
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let handler = handler.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| {
                    let handler = handler.clone();
                    async move {
                        let (parts, body) = request.into_parts();
                        // Local service discovery can probe newly opened test
                        // ports. Only fixture POSTs belong to the test workload.
                        if parts.method != hyper::Method::POST {
                            return Ok::<_, Infallible>(reply(404, "not a fixture request"));
                        }
                        let body = body.collect().await.unwrap().to_bytes();
                        Ok::<_, Infallible>(handler(parts.headers, body))
                    }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(socket), service)
                    .await;
            });
        }
    });
    Upstream { url, task }
}

fn fixture() -> (tempfile::TempDir, AgentAccounts) {
    let root = tempfile::tempdir().unwrap();
    let endpoints = ProbeEndpoints {
        claude_usage: "http://127.0.0.1:9/usage".into(),
        codex_usage: "http://127.0.0.1:9/usage".into(),
        ..Default::default()
    };
    let accounts = AgentAccounts::with_endpoints(
        AgentAccountsConfig::isolated(root.path()),
        endpoints,
        Default::default(),
    );
    (root, accounts)
}

fn slot(accounts: &AgentAccounts, harness: HarnessId, name: &str) -> Slot {
    let slot = Slot {
        id: slot_id_for(harness, name),
        harness,
        account_key: name.into(),
        profile: SlotProfile {
            email: format!("{name}@example.com"),
            display_name: None,
            organization: None,
            plan: None,
            auth_kind: AgentAuthKind::Oauth,
        },
        credentials: if harness == HarnessId::ClaudeCode {
            serde_json::json!({"claudeAiOauth":{"accessToken":name,"refreshToken":format!("refresh-{name}"),"expiresAt":4_102_444_800_000i64}})
        } else {
            serde_json::json!({"tokens":{"access_token":name,"refresh_token":format!("refresh-{name}"),"account_id":name}})
        },
        claude_config: Some(
            serde_json::json!({"oauthAccount":{"accountUuid":format!("account-{name}")}}),
        ),
        saved_at: now_ms(),
        created_at: None,
        store_key: None,
    };
    accounts.write_slot(&slot).unwrap();
    slot
}

fn window(label: &str, used: f32, after_ms: i64) -> AgentUsageWindow {
    AgentUsageWindow {
        label: label.into(),
        used_fraction: used,
        resets_at: DateTime::from_timestamp_millis(now_ms() + after_ms),
    }
}

fn quota(accounts: &AgentAccounts, slot: &Slot, windows: Vec<AgentUsageWindow>) {
    lock(&accounts.inner.pool.state).observations.insert(
        slot.id.clone(),
        Observation {
            windows,
            observed_at: now_ms(),
            ..Default::default()
        },
    );
}

fn enable(accounts: &AgentAccounts, harness: HarnessId, order: &[&Slot]) {
    let mut config = lock(&accounts.inner.pool.config);
    let provider = if harness == HarnessId::ClaudeCode {
        &mut config.claude
    } else {
        &mut config.codex
    };
    provider.enabled = true;
    provider.account_order = order.iter().map(|s| s.id.clone()).collect();
}

fn point_to(accounts: &AgentAccounts, upstream: &Upstream) {
    let mut endpoints = lock(&accounts.inner.pool.endpoints);
    endpoints.claude = upstream.url.clone();
    endpoints.codex = upstream.url.clone();
    endpoints.openai = upstream.url.clone();
}

#[test]
fn weekly_priority_falls_back_and_returns_after_session_reset_for_both_providers() {
    for harness in [HarnessId::ClaudeCode, HarnessId::Codex] {
        let (_root, accounts) = fixture();
        let a = slot(&accounts, harness, "a");
        let b = slot(&accounts, harness, "b");
        let unknown = slot(&accounts, harness, "unknown");
        enable(&accounts, harness, &[&a, &b, &unknown]);
        quota(&accounts, &a, vec![window("Week", 0.2, 300_000)]);
        quota(&accounts, &b, vec![window("Week", 0.2, 100_000)]);
        let select = |now| {
            accounts
                .pool_candidates(harness, "opus", Some("same-thread"), &HashSet::new(), now)
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            select(now_ms()),
            [b.id.clone(), a.id.clone(), unknown.id.clone()]
        );
        let reset = now_ms() + 50_000;
        quota(
            &accounts,
            &b,
            vec![
                window("Week", 0.2, 100_000),
                AgentUsageWindow {
                    label: "Session".into(),
                    used_fraction: 0.98,
                    resets_at: DateTime::from_timestamp_millis(reset),
                },
            ],
        );
        assert_eq!(select(now_ms())[0], a.id);
        assert_eq!(select(reset + 1)[0], b.id);
        // Once the weekly reset itself is past, unknowns return to priority order.
        assert_eq!(select(now_ms() + 400_000), [a.id, b.id, unknown.id]);
    }
}

#[test]
fn family_limits_do_not_exhaust_the_other_families_and_month_is_not_a_week() {
    let windows = vec![
        window("Week", 0.1, 500_000),
        window("Week (opus)", 1.0, 10_000),
    ];
    assert!(!eligible(&windows, "opus", 0.98, now_ms()));
    assert!(eligible(&windows, "sonnet", 0.98, now_ms()));
    assert_eq!(
        weekly_reset(&windows, "opus", now_ms()),
        windows[1].resets_at.map(|at| at.timestamp_millis())
    );
    assert!(weekly_reset(&[window("Month", 0.1, 500_000)], "other", now_ms()).is_none());
}

#[test]
fn usage_probe_parses_scoped_limits_and_string_percentages() {
    let usage = claude_usage_windows(&serde_json::json!({
        "five_hour":{"used_percentage":"10","reset_at":1800000000},
        "seven_day_opus":{"utilization":35,"resets_at":"2027-01-01T00:00:00Z"},
        "limits":[
            {"kind":"weekly_scoped","scope":{"model":{"display_name":"Claude Opus 5.5"}},"percent":"99","resets_at":1800001000},
            {"group":"weekly","scope":{"model":{"display_name":"Claude Sonnet 5.5"}},"percent":"20","resets_at":1800002000}
        ]
    })).unwrap();
    assert_eq!(usage.windows.len(), 3);
    assert_eq!(
        usage
            .windows
            .iter()
            .find(|w| w.label == "Week (opus)")
            .unwrap()
            .used_fraction,
        0.99
    );
    assert_eq!(usage.windows[0].resets_at.unwrap().timestamp(), 1800000000);
}

#[tokio::test]
async fn cancel_stream_closes_upstream_and_releases_inflight_promptly() {
    struct Dropped(Arc<AtomicUsize>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let (_root, accounts) = fixture();
    let a = slot(&accounts, HarnessId::Codex, "a");
    enable(&accounts, a.harness, &[&a]);
    let dropped = Arc::new(AtomicUsize::new(0));
    let observed = dropped.clone();
    let upstream = upstream(move |_, _| {
        let guard = Dropped(observed.clone());
        let stream = futures::stream::unfold((false, guard), |(sent, guard)| async move {
            if sent {
                futures::future::pending::<()>().await;
            }
            Some((
                Ok::<_, std::io::Error>(Frame::data(Bytes::from_static(b"data: first\n\n"))),
                (true, guard),
            ))
        });
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(StreamBody::new(stream).boxed_unsync())
            .unwrap()
    })
    .await;
    point_to(&accounts, &upstream);
    let route = accounts
        .pool_route(a.harness, "chat")
        .await
        .unwrap()
        .unwrap();
    let response = reqwest::Client::new()
        .post(format!("{}/responses", route.base_url))
        .bearer_auth(route.token)
        .body("{}")
        .send()
        .await
        .unwrap();
    let mut stream = response.bytes_stream();
    assert!(stream.next().await.unwrap().is_ok());
    drop(stream);
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if dropped.load(Ordering::SeqCst) == 1
                && accounts
                    .pool_snapshot()
                    .accounts
                    .iter()
                    .all(|a| a.in_flight == 0)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("client disconnect must cancel upstream");
    accounts.shutdown();
}

#[tokio::test]
async fn proxy_authenticates_injects_credentials_rewrites_identity_and_preserves_stream() {
    let (_root, accounts) = fixture();
    let slot = slot(&accounts, HarnessId::ClaudeCode, "private-access-token");
    enable(&accounts, slot.harness, &[&slot]);
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let upstream = upstream(move |headers, bytes| {
        calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(headers["authorization"], "Bearer private-access-token");
        assert!(!headers.contains_key("x-api-key"));
        assert!(!headers.contains_key("x-secret-header"));
        assert!(
            headers["anthropic-beta"]
                .to_str()
                .unwrap()
                .contains("oauth-2025-04-20")
        );
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let user: serde_json::Value =
            serde_json::from_str(value["metadata"]["user_id"].as_str().unwrap()).unwrap();
        assert_eq!(user["account_uuid"], "account-private-access-token");
        let mut response = reply(200, "event: message\ndata: hello\n\n");
        response
            .headers_mut()
            .insert("content-type", "text/event-stream".parse().unwrap());
        response
    })
    .await;
    point_to(&accounts, &upstream);
    let route = accounts
        .pool_route(slot.harness, "chat")
        .await
        .unwrap()
        .unwrap();
    let url = format!("{}/v1/messages", route.base_url);
    let client = reqwest::Client::new();
    assert_eq!(
        client.post(&url).body("{}").send().await.unwrap().status(),
        401
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth("wrong")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{}/bad", route.base_url))
            .bearer_auth(&route.token)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client
            .get(&url)
            .bearer_auth(&route.token)
            .header("upgrade", "websocket")
            .send()
            .await
            .unwrap()
            .status(),
        426
    );
    let response = client
        .post(&url)
        .bearer_auth(&route.token)
        .header("x-api-key", "inbound-must-not-escape")
        .header("x-secret-header", "private")
        .json(&serde_json::json!({"model":"claude-opus-5-5",
            "metadata":{"user_id":"{\"account_uuid\":\"wrong\",\"session_id\":\"conversation\"}"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.text().await.unwrap(),
        "event: message\ndata: hello\n\n"
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(route.base_url.starts_with("http://127.0.0.1:"));
    let snapshot = serde_json::to_string(&accounts.pool_snapshot()).unwrap();
    for secret in [
        "private-access-token",
        "refresh-private-access-token",
        route.token.as_str(),
    ] {
        assert!(!snapshot.contains(secret));
    }
    assert!(!format!("{route:?}").contains(&route.token));
    accounts.shutdown();
}

#[tokio::test]
async fn quota_429_falls_back_and_family_rejection_does_not_poison_sonnet() {
    let (_root, accounts) = fixture();
    let a = slot(&accounts, HarnessId::ClaudeCode, "a");
    let b = slot(&accounts, a.harness, "b");
    enable(&accounts, a.harness, &[&a, &b]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let upstream = upstream(move |headers, bytes| {
        let auth = headers["authorization"].to_str().unwrap();
        seen.lock().unwrap().push(auth.to_owned());
        let opus = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["model"]
            .as_str()
            .unwrap()
            .contains("opus");
        if auth == "Bearer a" && opus {
            let mut response = reply(429, "{}");
            response.headers_mut().insert(
                "anthropic-ratelimit-unified-7d_opus-status",
                "rejected".parse().unwrap(),
            );
            response.headers_mut().insert(
                "anthropic-ratelimit-unified-7d_opus-reset",
                ((now_ms() / 1000) + 3600).to_string().parse().unwrap(),
            );
            response
        } else {
            reply(200, "ok")
        }
    })
    .await;
    point_to(&accounts, &upstream);
    let route = accounts
        .pool_route(a.harness, "chat")
        .await
        .unwrap()
        .unwrap();
    let send = |model| {
        reqwest::Client::new()
            .post(format!("{}/v1/messages", route.base_url))
            .bearer_auth(&route.token)
            .json(&serde_json::json!({"model":model}))
            .send()
    };
    assert_eq!(
        send("claude-opus-5-5").await.unwrap().text().await.unwrap(),
        "ok"
    );
    assert_eq!(
        send("claude-sonnet-5-5")
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    assert_eq!(*calls.lock().unwrap(), ["Bearer a", "Bearer b", "Bearer a"]);
    accounts.shutdown();
}

#[tokio::test]
async fn errors_fall_back_but_the_next_request_returns_to_the_preferred_account() {
    let (_root, accounts) = fixture();
    let a = slot(&accounts, HarnessId::Codex, "a");
    let b = slot(&accounts, a.harness, "b");
    enable(&accounts, a.harness, &[&a, &b]);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let upstream = upstream(move |headers, _| {
        let call = count.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            reply(503, "error")
        } else {
            assert_eq!(
                headers["chatgpt-account-id"],
                if call == 1 { "b" } else { "a" }
            );
            reply(200, "ok")
        }
    })
    .await;
    point_to(&accounts, &upstream);
    let route = accounts
        .pool_route(a.harness, "chat")
        .await
        .unwrap()
        .unwrap();
    for _ in 0..2 {
        let response = reqwest::Client::new()
            .post(format!("{}/responses", route.base_url))
            .bearer_auth(&route.token)
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "ok");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    accounts.shutdown();
}

#[tokio::test]
async fn broken_stream_is_never_replayed_and_releases_inflight() {
    let (_root, accounts) = fixture();
    let a = slot(&accounts, HarnessId::Codex, "a");
    let b = slot(&accounts, a.harness, "b");
    enable(&accounts, a.harness, &[&a, &b]);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let upstream = upstream(move |_, _| {
        count.fetch_add(1, Ordering::SeqCst);
        let stream = futures::stream::once(async {
            Ok(Frame::data(Bytes::from_static(b"data: first\n\n")))
        })
        .chain(futures::stream::once(async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            Err(std::io::Error::other("test stream disconnect"))
        }));
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(StreamBody::new(stream).boxed_unsync())
            .unwrap()
    })
    .await;
    point_to(&accounts, &upstream);
    let route = accounts
        .pool_route(a.harness, "chat")
        .await
        .unwrap()
        .unwrap();
    let response = reqwest::Client::new()
        .post(format!("{}/responses", route.base_url))
        .bearer_auth(route.token)
        .body("{}")
        .send()
        .await
        .unwrap();
    let text = response.text().await.unwrap();
    assert!(text.starts_with("data: first\n\n"));
    assert_eq!(text.matches("event: error").count(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        accounts
            .pool_snapshot()
            .accounts
            .iter()
            .all(|a| a.in_flight == 0)
    );
    accounts.shutdown();
}

#[tokio::test]
async fn refresh_is_singleflight_and_never_changes_live_store() {
    let (root, accounts) = fixture();
    let mut saved = slot(&accounts, HarnessId::ClaudeCode, "inactive");
    saved.credentials["claudeAiOauth"]["expiresAt"] = (now_ms() + 1000).into();
    accounts.write_slot(&saved).unwrap();
    let live = accounts.inner.config.claude_creds_file();
    std::fs::create_dir_all(live.parent().unwrap()).unwrap();
    std::fs::write(&live, b"{\"unrelated-live-store\":true}").unwrap();
    let before = std::fs::read(&live).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let upstream = upstream(move |_, _| {
        count.fetch_add(1, Ordering::SeqCst);
        reply(
            200,
            "{\"access_token\":\"refreshed\",\"refresh_token\":\"rotated\",\"expires_in\":3600}",
        )
    })
    .await;
    // The ordinary token endpoint is immutable in production; construct an
    // isolated accounts instance with its test endpoints pointed at the fixture.
    let mut endpoints = ProbeEndpoints::default();
    endpoints.claude_token = upstream.url.clone();
    let accounts =
        AgentAccounts::with_endpoints(accounts.inner.config.clone(), endpoints, Default::default());
    let (first, second) = tokio::join!(
        accounts.pool_secret(&saved, false),
        accounts.pool_secret(&saved, false)
    );
    assert_eq!(
        first.unwrap().credentials["claudeAiOauth"]["accessToken"],
        "refreshed"
    );
    assert_eq!(
        second.unwrap().credentials["claudeAiOauth"]["accessToken"],
        "refreshed"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read(live).unwrap(), before);
    accounts.list(false).await.unwrap();
    assert_eq!(
        accounts
            .read_slot(saved.harness, &saved.id)
            .unwrap()
            .credentials["claudeAiOauth"]["refreshToken"],
        "rotated"
    );
    drop(root);
}

#[tokio::test]
async fn expired_live_token_is_read_only_and_bypass_stays_local() {
    let (_root, accounts) = fixture();
    let mut live = slot(&accounts, HarnessId::ClaudeCode, "live");
    live.credentials["claudeAiOauth"]["expiresAt"] = (now_ms() - 1000).into();
    accounts.write_slot(&live).unwrap();
    let config = &accounts.inner.config;
    std::fs::create_dir_all(&config.claude_config_dir).unwrap();
    std::fs::write(config.claude_creds_file(), live.credentials.to_string()).unwrap();
    std::fs::write(&config.claude_config_file, serde_json::json!({"oauthAccount":{"accountUuid":"live","emailAddress":"live@example.com"}}).to_string()).unwrap();
    let before = std::fs::read(config.claude_creds_file()).unwrap();
    assert!(
        accounts
            .pool_secret(&live, false)
            .await
            .unwrap_err()
            .message()
            .starts_with("Live login expired")
    );
    assert_eq!(before, std::fs::read(config.claude_creds_file()).unwrap());
    enable(&accounts, live.harness, &[&live]);
    accounts.set_pool_bypass("bypass", true).await.unwrap();
    assert!(
        accounts
            .pool_route(live.harness, "bypass")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        accounts
            .pool_route(HarnessId::Cursor, "other")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn priority_and_sequential_bindings_survive_restart() {
    let (_root, accounts) = fixture();
    let a = slot(&accounts, HarnessId::Codex, "a");
    let b = slot(&accounts, a.harness, "b");
    let settings = AccountPoolProviderConfig {
        enabled: true,
        routing_mode: AccountPoolRoutingMode::Sequential,
        account_order: vec![b.id.clone(), a.id.clone()],
        ..Default::default()
    };
    accounts.configure_pool(a.harness, settings).await.unwrap();
    accounts.accept_pool_account(&a, "other", Some("session"));
    let restarted = AgentAccounts::new(accounts.inner.config.clone());
    let snapshot = restarted.list(false).await.unwrap();
    assert_eq!(
        snapshot.accounts.iter().map(|a| &a.id).collect::<Vec<_>>(),
        [&b.id, &a.id]
    );
    assert_eq!(
        restarted.pool_candidates(
            a.harness,
            "other",
            Some("session"),
            &HashSet::new(),
            now_ms()
        )[0]
        .id,
        a.id
    );
}

#[tokio::test]
async fn api_key_import_is_private_idempotent_and_never_becomes_the_live_login() {
    let (_root, accounts) = fixture();
    let snapshot = accounts
        .add_pool_api_key(
            HarnessId::ClaudeCode,
            "test-private-api-key",
            Some("Work key"),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.accounts.len(), 1);
    let id = snapshot.accounts[0].id.clone();
    assert!(!snapshot.accounts[0].active);
    assert_eq!(snapshot.accounts[0].auth_kind, Some(AgentAuthKind::ApiKey));
    assert!(snapshot.pool.accounts[0].poolable);
    let repeated = accounts
        .add_pool_api_key(HarnessId::ClaudeCode, "test-private-api-key", None)
        .await
        .unwrap();
    assert_eq!(repeated.accounts.len(), 1);
    assert_eq!(repeated.accounts[0].id, id);
    assert!(
        !serde_json::to_string(&repeated)
            .unwrap()
            .contains("test-private-api-key")
    );
    assert!(!accounts.inner.config.claude_creds_file().exists());
    assert!(accounts.activate(HarnessId::ClaudeCode, &id).await.is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = accounts
            .slots_dir(HarnessId::ClaudeCode)
            .unwrap()
            .join(format!("{id}.json"));
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn healthy_requests_do_not_wait_for_the_live_store_gate() {
    let (_root, accounts) = fixture();
    let saved = slot(&accounts, HarnessId::ClaudeCode, "healthy");
    let _ops = accounts.inner.ops.lock().await;
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        accounts.pool_secret(&saved, false),
    )
    .await;
    assert!(
        result.unwrap().is_ok(),
        "ordinary requests must not block on a settings refresh"
    );
}

#[tokio::test]
async fn temporary_refresh_failures_back_off_even_after_access_token_expiry() {
    let (_root, initial) = fixture();
    let mut saved = slot(&initial, HarnessId::ClaudeCode, "expiring");
    saved.credentials["claudeAiOauth"]["expiresAt"] = (now_ms() + 60_000).into();
    initial.write_slot(&saved).unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let upstream = upstream(move |_, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        reply(503, "temporary outage")
    })
    .await;
    let mut endpoints = initial.inner.endpoints.clone();
    endpoints.claude_token = upstream.url.clone();
    let accounts =
        AgentAccounts::with_endpoints(initial.inner.config.clone(), endpoints, Default::default());
    assert!(
        accounts.pool_secret(&saved, false).await.is_ok(),
        "unexpired access token remains usable"
    );
    assert!(accounts.pool_secret(&saved, false).await.is_ok());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    saved.credentials["claudeAiOauth"]["expiresAt"] = (now_ms() - 1000).into();
    accounts.write_slot(&saved).unwrap();
    let (one, two) = tokio::join!(
        accounts.pool_secret(&saved, false),
        accounts.pool_secret(&saved, false)
    );
    assert!(matches!(one, Err(SecretError::Temporary(_))));
    assert!(matches!(two, Err(SecretError::Temporary(_))));
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "expired tokens must still respect refresh backoff"
    );
}

#[tokio::test]
async fn persistent_401_records_the_rotated_credentials_without_refresh_churn() {
    let (_root, initial) = fixture();
    let a = slot(&initial, HarnessId::ClaudeCode, "a");
    let b = slot(&initial, a.harness, "b");
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let upstream = upstream(move |headers, bytes| {
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        if body["grant_type"] == "refresh_token" {
            calls.fetch_add(1, Ordering::SeqCst);
            reply(200, "{\"access_token\":\"still-rejected\",\"refresh_token\":\"rotated\",\"expires_in\":3600}")
        } else if headers["authorization"] == "Bearer b" { reply(200, "ok") }
        else { reply(401, "rejected") }
    }).await;
    let mut endpoints = initial.inner.endpoints.clone();
    endpoints.claude_token = format!("{}/token", upstream.url);
    let accounts =
        AgentAccounts::with_endpoints(initial.inner.config.clone(), endpoints, Default::default());
    enable(&accounts, a.harness, &[&a, &b]);
    point_to(&accounts, &upstream);
    let route = accounts
        .pool_route(a.harness, "chat")
        .await
        .unwrap()
        .unwrap();
    for _ in 0..2 {
        let response = reqwest::Client::new()
            .post(format!("{}/v1/messages", route.base_url))
            .bearer_auth(&route.token)
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "ok");
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        accounts
            .pool_snapshot()
            .accounts
            .iter()
            .find(|s| s.account_id == a.id)
            .unwrap()
            .status,
        AccountPoolStatus::NeedsSignIn
    );
    accounts.shutdown();
}

#[test]
fn rotated_credentials_survive_a_temporary_slot_write_failure_in_memory() {
    let (_root, accounts) = fixture();
    let mut saved = slot(&accounts, HarnessId::ClaudeCode, "saved");
    let path = accounts
        .slots_dir(saved.harness)
        .unwrap()
        .join(format!("{}.json", saved.id));
    let backup = path.with_extension("backup");
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    saved.credentials["claudeAiOauth"]["accessToken"] = "fresh".into();
    saved.credentials["claudeAiOauth"]["refreshToken"] = "single-use-rotated".into();
    accounts.save_refreshed_pool_slot(&saved);
    assert_eq!(
        accounts
            .read_slot(saved.harness, &saved.id)
            .unwrap()
            .credentials["claudeAiOauth"]["refreshToken"],
        "single-use-rotated"
    );
    assert_eq!(accounts.read_slots(saved.harness).len(), 1);
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(backup, &path).unwrap();
    accounts.retry_pool_credential_saves();
    assert!(
        accounts
            .pending_pool_slot(saved.harness, &saved.id)
            .is_none()
    );
    assert_eq!(
        accounts
            .read_slot(saved.harness, &saved.id)
            .unwrap()
            .credentials["claudeAiOauth"]["refreshToken"],
        "single-use-rotated"
    );
}

#[test]
fn priority_fallback_advances_after_the_cursor_and_wraps() {
    let (_root, accounts) = fixture();
    let a = slot(&accounts, HarnessId::Codex, "a");
    let b = slot(&accounts, a.harness, "b");
    let c = slot(&accounts, a.harness, "c");
    enable(&accounts, a.harness, &[&a, &b, &c]);
    lock(&accounts.inner.pool.config).codex.routing_mode = AccountPoolRoutingMode::Sequential;
    accounts.accept_pool_account(&b, "other", Some("thread"));
    quota(&accounts, &b, vec![window("Session", 1.0, 100_000)]);
    let candidates = accounts.pool_candidates(
        a.harness,
        "other",
        Some("thread"),
        &HashSet::new(),
        now_ms(),
    );
    assert_eq!(
        candidates.iter().map(|s| &s.id).collect::<Vec<_>>(),
        [&c.id, &a.id]
    );
}

#[test]
fn snapshot_exposes_model_specific_next_accounts_without_reranking_the_rows() {
    let (_root, accounts) = fixture();
    let a = slot(&accounts, HarnessId::ClaudeCode, "a");
    let b = slot(&accounts, a.harness, "b");
    enable(&accounts, a.harness, &[&a, &b]);
    quota(
        &accounts,
        &a,
        vec![
            window("Week", 0.1, 50_000),
            window("Week (opus)", 1.0, 100_000),
        ],
    );
    quota(&accounts, &b, vec![window("Week", 0.1, 200_000)]);
    let snapshot = accounts.pool_snapshot();
    let provider = snapshot
        .providers
        .iter()
        .find(|p| p.harness == a.harness)
        .unwrap();
    assert_eq!(provider.next_account_id.as_ref(), Some(&a.id));
    assert_eq!(provider.next_account_ids.get("sonnet"), Some(&a.id));
    assert_eq!(provider.next_account_ids.get("opus"), Some(&b.id));
    assert_eq!(snapshot.config.claude.account_order, [a.id, b.id]);
}

#[test]
fn legacy_claude_identity_rewrites_both_account_component_positions() {
    let (_root, accounts) = fixture();
    let saved = slot(&accounts, HarnessId::ClaudeCode, "selected");
    for prefix in ["", "user_old_"] {
        let value = serde_json::json!({"metadata":{"user_id":format!("{prefix}account_00000000-0000-0000-0000-000000000000_session_keep")}});
        let raw = Bytes::from(value.to_string());
        let rewritten = http::rewrite_body(saved.harness, &value, &raw, &saved);
        let value: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(
            value["metadata"]["user_id"],
            format!("{prefix}account_account-selected_session_keep")
        );
    }
}

#[test]
fn zero_claude_expiry_metadata_is_unknown_instead_of_an_expired_login() {
    let (_root, accounts) = fixture();
    let mut saved = slot(&accounts, HarnessId::ClaudeCode, "fresh");
    saved.credentials["claudeAiOauth"]["expiresAt"] = 0.into();
    assert_eq!(token_expiry(&saved), None);
}

#[tokio::test]
async fn refresh_survives_canceled_waiters_and_shutdown_drains_persistence() {
    for harness in [HarnessId::ClaudeCode, HarnessId::Codex] {
        let (_root, initial) = fixture();
        let saved = slot(&initial, harness, "cancel-refresh");
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let count = Arc::new(AtomicUsize::new(0));
        let (signal, finish, calls) = (started.clone(), release.clone(), count.clone());
        let upstream = upstream(move |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            signal.notify_one();
            let finish = finish.clone();
            Response::new(StreamBody::new(futures::stream::once(async move {
                finish.notified().await;
                Ok::<_, std::io::Error>(Frame::data(Bytes::from_static(
                    b"{\"access_token\":\"fresh-after-cancel\",\"refresh_token\":\"rotated-after-cancel\",\"expires_in\":3600}"
                )))
            })).boxed_unsync())
        }).await;
        let mut endpoints = initial.inner.endpoints.clone();
        endpoints.claude_token = upstream.url.clone();
        let accounts = AgentAccounts::with_endpoints(
            initial.inner.config.clone(),
            endpoints,
            Default::default(),
        );
        lock(&accounts.inner.pool.endpoints).codex_refresh = upstream.url.clone();
        let waiter = tokio::spawn({
            let accounts = accounts.clone();
            let saved = saved.clone();
            async move { accounts.pool_secret(&saved, true).await }
        });
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .unwrap();
        waiter.abort();
        let _ = waiter.await;
        let second = tokio::spawn({
            let accounts = accounts.clone();
            let saved = saved.clone();
            async move { accounts.pool_secret(&saved, true).await }
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let queued = lock(&accounts.inner.pool.refresh_tasks)
                    .as_ref()
                    .unwrap()
                    .len()
                    >= 2;
                if queued {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let shutdown = tokio::spawn({
            let accounts = accounts.clone();
            async move {
                accounts.drain_pool_refreshes().await;
            }
        });
        tokio::task::yield_now().await;
        assert!(
            !shutdown.is_finished(),
            "shutdown waits for the consumed refresh to be persisted"
        );
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), shutdown)
            .await
            .unwrap()
            .unwrap();
        assert!(second.await.unwrap().is_ok());
        let fresh = accounts.read_slot(harness, &saved.id).unwrap();
        let refresh = if harness == HarnessId::ClaudeCode {
            &fresh.credentials["claudeAiOauth"]["refreshToken"]
        } else {
            &fresh.credentials["tokens"]["refresh_token"]
        };
        assert_eq!(refresh, "rotated-after-cancel");
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "single-use token is posted once"
        );
        accounts.shutdown();
    }
}

#[test]
fn reset_only_headers_do_not_publish_unknown_usage_as_zero() {
    let (_root, accounts) = fixture();
    let saved = slot(&accounts, HarnessId::ClaudeCode, "unknown");
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "anthropic-ratelimit-unified-7d-reset",
        ((now_ms() + 60_000) / 1000).to_string().parse().unwrap(),
    );
    http::observe(&accounts, &saved, &headers, "opus");
    assert!(
        accounts
            .pool_windows(&saved, &Observation::default())
            .is_empty()
    );
    assert!(accounts.list_pool_test_windows(&saved).is_empty());
    headers.insert(
        "anthropic-ratelimit-unified-7d-status",
        "rejected".parse().unwrap(),
    );
    http::observe(&accounts, &saved, &headers, "opus");
    assert_eq!(
        accounts.list_pool_test_windows(&saved)[0].used_fraction,
        1.0
    );
}

impl AgentAccounts {
    fn list_pool_test_windows(&self, saved: &Slot) -> Vec<AgentUsageWindow> {
        let observation = lock(&self.inner.pool.state)
            .observations
            .get(&saved.id)
            .cloned()
            .unwrap_or_default();
        self.pool_windows(saved, &observation)
    }
}

#[test]
fn partial_codex_headers_preserve_bucket_identity_and_do_not_invent_weekly_windows() {
    let (_root, accounts) = fixture();
    let saved = slot(&accounts, HarnessId::Codex, "monthly");
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-codex-primary-window-minutes", "43200".parse().unwrap());
    headers.insert("x-codex-primary-used-percent", "99".parse().unwrap());
    headers.insert(
        "x-codex-primary-reset-at",
        ((now_ms() + 60_000) / 1000).to_string().parse().unwrap(),
    );
    http::observe(&accounts, &saved, &headers, "other");
    assert_eq!(accounts.list_pool_test_windows(&saved)[0].label, "Month");
    let mut partial = reqwest::header::HeaderMap::new();
    partial.insert("x-codex-primary-used-percent", "20".parse().unwrap());
    http::observe(&accounts, &saved, &partial, "other");
    let windows = accounts.list_pool_test_windows(&saved);
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].label, "Month");
    assert_eq!(windows[0].used_fraction, 0.2);
    // A duration change must remove the old label, including older usage-cache data.
    lock(&accounts.inner.usage).insert(
        usage_key(saved.harness, &saved.account_key),
        UsageEntry {
            usage: Some(UsageSnapshot {
                windows: vec![window("Month", 1.0, 60_000)],
                ..Default::default()
            }),
            fetched_at: Some(now_ms() - 1000),
            ..Default::default()
        },
    );
    partial.insert("x-codex-primary-window-minutes", "300".parse().unwrap());
    http::observe(&accounts, &saved, &partial, "other");
    assert!(
        accounts
            .list_pool_test_windows(&saved)
            .iter()
            .all(|window| window.label != "Month")
    );
    let unknown = slot(&accounts, HarnessId::Codex, "unknown-duration");
    let mut secondary = reqwest::header::HeaderMap::new();
    secondary.insert("x-codex-secondary-used-percent", "80".parse().unwrap());
    secondary.insert(
        "x-codex-secondary-reset-at",
        ((now_ms() + 60_000) / 1000).to_string().parse().unwrap(),
    );
    http::observe(&accounts, &unknown, &secondary, "other");
    let windows = accounts.list_pool_test_windows(&unknown);
    assert_eq!(windows[0].label, "Secondary");
    assert!(weekly_reset(&windows, "other", now_ms()).is_none());
    accounts.flush_pool();
    let reloaded = AgentAccounts::with_endpoints(
        accounts.inner.config.clone(),
        accounts.inner.endpoints.clone(),
        Default::default(),
    );
    let mut partial = reqwest::header::HeaderMap::new();
    partial.insert("x-codex-primary-used-percent", "30".parse().unwrap());
    http::observe(&reloaded, &saved, &partial, "other");
    assert_eq!(reloaded.list_pool_test_windows(&saved)[0].label, "Session");
}

#[test]
fn rejection_only_claude_probe_buckets_block_only_the_affected_family() {
    let usage=claude_usage_windows(&serde_json::json!({
        "seven_day": {"utilization": 10},
        "seven_day_opus": {"status":"rejected", "resets_at":(Utc::now()+chrono::TimeDelta::hours(1)).to_rfc3339()}
    })).unwrap();
    assert!(!eligible(&usage.windows, "opus", 0.98, now_ms()));
    assert!(eligible(&usage.windows, "sonnet", 0.98, now_ms()));
    let only =
        claude_usage_windows(&serde_json::json!({"five_hour":{"status":"rejected"}})).unwrap();
    assert_eq!(only.windows[0].used_fraction, 1.0);
}

#[tokio::test]
async fn live_expiry_can_recover_after_ownership_changes_without_clearing_auth_failures() {
    let (_root, accounts) = fixture();
    let mut a = slot(&accounts, HarnessId::ClaudeCode, "a");
    let b = slot(&accounts, a.harness, "b");
    a.credentials["claudeAiOauth"]["expiresAt"] = (now_ms() - 1000).into();
    accounts.write_slot(&a).unwrap();
    enable(&accounts, a.harness, &[&a, &b]);
    std::fs::create_dir_all(accounts.inner.config.claude_config_file.parent().unwrap()).unwrap();
    std::fs::write(
        &accounts.inner.config.claude_config_file,
        serde_json::json!({"oauthAccount":{"accountUuid":"a"}}).to_string(),
    )
    .unwrap();
    http::mark_error(
        &accounts,
        &a,
        "Live login expired; sign in again in the CLI",
    );
    assert_eq!(
        accounts.pool_candidates(a.harness, "other", None, &HashSet::new(), now_ms())[0].id,
        b.id
    );
    std::fs::write(
        &accounts.inner.config.claude_config_file,
        serde_json::json!({"oauthAccount":{"accountUuid":"b"}}).to_string(),
    )
    .unwrap();
    assert_eq!(
        accounts.pool_candidates(a.harness, "other", None, &HashSet::new(), now_ms())[0].id,
        a.id
    );
    http::mark_error(&accounts, &a, "Needs sign-in");
    assert_eq!(
        accounts.pool_candidates(a.harness, "other", None, &HashSet::new(), now_ms())[0].id,
        b.id
    );
}

#[tokio::test]
async fn an_unsaved_unreadable_live_login_does_not_enter_configurable_pool_order() {
    let (_root, accounts) = fixture();
    let saved = slot(&accounts, HarnessId::ClaudeCode, "saved");
    std::fs::create_dir_all(accounts.inner.config.claude_config_file.parent().unwrap()).unwrap();
    std::fs::write(&accounts.inner.config.claude_config_file, serde_json::json!({"oauthAccount":{"accountUuid":"unreadable","emailAddress":"unreadable@example.com"}}).to_string()).unwrap();
    let snapshot = accounts.list(false).await.unwrap();
    assert_eq!(
        snapshot.pool.config.claude.account_order,
        [saved.id.clone()]
    );
    let unreadable = snapshot
        .accounts
        .iter()
        .find(|account| account.email.as_deref() == Some("unreadable@example.com"))
        .unwrap();
    assert!(
        !snapshot
            .pool
            .accounts
            .iter()
            .find(|status| status.account_id == unreadable.id)
            .unwrap()
            .poolable
    );
    let mut config = snapshot.pool.config.claude;
    config.enabled = true;
    assert!(
        accounts
            .configure_pool(HarnessId::ClaudeCode, config)
            .await
            .is_ok()
    );
}

#[test]
fn codex_buckets_with_the_same_display_label_keep_independent_limits() {
    let (_root, accounts) = fixture();
    let saved = slot(&accounts, HarnessId::Codex, "two-short-windows");
    let mut headers = reqwest::header::HeaderMap::new();
    for (bucket, minutes, used) in [("primary", "300", "100"), ("secondary", "1440", "10")] {
        headers.insert(
            format!("x-codex-{bucket}-window-minutes")
                .parse::<reqwest::header::HeaderName>()
                .unwrap(),
            minutes.parse().unwrap(),
        );
        headers.insert(
            format!("x-codex-{bucket}-used-percent")
                .parse::<reqwest::header::HeaderName>()
                .unwrap(),
            used.parse().unwrap(),
        );
    }
    http::observe(&accounts, &saved, &headers, "other");
    let windows = accounts.list_pool_test_windows(&saved);
    assert_eq!(windows.len(), 2);
    assert!(windows.iter().all(|window| window.label == "Session"));
    assert!(
        !eligible(&windows, "other", 0.98, now_ms()),
        "the secondary bucket cannot overwrite the exhausted primary"
    );
    let mut partial = reqwest::header::HeaderMap::new();
    partial.insert("x-codex-primary-used-percent", "0".parse().unwrap());
    http::observe(&accounts, &saved, &partial, "other");
    let windows = accounts.list_pool_test_windows(&saved);
    assert_eq!(windows.len(), 2);
    assert!(eligible(&windows, "other", 0.98, now_ms()));
    assert_eq!(windows[1].used_fraction, 0.1);
}

#[test]
fn reset_only_headers_do_not_carry_exhaustion_into_a_new_period() {
    let (_root, accounts) = fixture();
    let saved = slot(&accounts, HarnessId::ClaudeCode, "reset-period");
    quota(&accounts, &saved, vec![window("Week", 1.0, -1000)]);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "anthropic-ratelimit-unified-7d-reset",
        ((now_ms() + 60_000) / 1000).to_string().parse().unwrap(),
    );
    http::observe(&accounts, &saved, &headers, "opus");
    assert!(accounts.list_pool_test_windows(&saved).is_empty());
}
