use std::convert::Infallible;
use std::error::Error;

use futures::StreamExt;
use http_body_util::{BodyExt, Full, Limited, StreamBody, combinators::UnsyncBoxBody};
use hyper::{
    Request, Response, StatusCode,
    body::{Bytes, Frame, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};

use super::*;

type Body = UnsyncBoxBody<Bytes, Box<dyn Error + Send + Sync>>;
const MAX_BODY: usize = 64 * 1024 * 1024;

pub(super) fn start(
    listener: tokio::net::TcpListener,
    inner: Weak<Inner>,
    token: String,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
        let capacity = Arc::new(tokio::sync::Semaphore::new(128));
        loop {
            let accepted = tokio::select! {
                _ = cancel.cancelled() => break,
                accepted = listener.accept() => accepted,
            };
            let socket = match accepted {
                Ok((socket, _)) => socket,
                Err(_) => {
                    tracing::warn!("account pool listener temporarily unavailable");
                    tokio::select! { _ = cancel.cancelled() => break, _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
                    continue;
                }
            };
            let Ok(permit) = capacity.clone().try_acquire_owned() else {
                continue;
            };
            let (inner, token, cancel) = (inner.clone(), token.clone(), cancel.clone());
            tokio::spawn(async move {
                let _permit = permit;
                let service = service_fn(move |request| {
                    let (inner, token) = (inner.clone(), token.clone());
                    async move {
                        let response = match inner.upgrade() {
                            Some(inner) => forward(AgentAccounts { inner }, request, &token).await,
                            None => error(
                                StatusCode::SERVICE_UNAVAILABLE,
                                "Account pool is shutting down",
                            ),
                        };
                        Ok::<_, Infallible>(response)
                    }
                });
                let mut server = http1::Builder::new();
                server
                    .timer(TokioTimer::new())
                    .header_read_timeout(Duration::from_secs(15));
                tokio::select! {
                    _ = cancel.cancelled() => {},
                    _ = server.serve_connection(TokioIo::new(socket), service) => {},
                }
            });
        }
    });
}

fn token_matches(left: &str, right: &str) -> bool {
    // Fixed-length digests avoid leaking either capability's length/content.
    let left = Sha256::digest(left.as_bytes());
    let right = Sha256::digest(right.as_bytes());
    left.iter()
        .zip(right.iter())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

fn error(status: StatusCode, message: &str) -> Response<Body> {
    let kind = if status == StatusCode::TOO_MANY_REQUESTS {
        "rate_limit_error"
    } else if status == StatusCode::UNAUTHORIZED {
        "authentication_error"
    } else {
        "api_error"
    };
    let bytes = Bytes::from(
        serde_json::json!({"type":"error","error":{"type":kind,"message":message}}).to_string(),
    );
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(
            Full::new(bytes)
                .map_err(|never| match never {})
                .boxed_unsync(),
        )
        .unwrap()
}

fn route(path: &str) -> Option<(HarnessId, String)> {
    if matches!(
        path,
        "/claude/v1/messages" | "/claude/v1/messages/count_tokens"
    ) {
        Some((
            HarnessId::ClaudeCode,
            path.trim_start_matches("/claude").into(),
        ))
    } else {
        let path = path.strip_prefix("/codex/v1/")?;
        (matches!(
            path,
            "responses" | "responses/compact" | "models" | "alpha/search"
        ) || path.starts_with("images/") && !path.contains("..") && !path.contains('%'))
        .then(|| (HarnessId::Codex, format!("/{path}")))
    }
}

async fn forward(
    accounts: AgentAccounts,
    request: Request<Incoming>,
    token: &str,
) -> Response<Body> {
    let auth = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .or_else(|| {
            request
                .headers()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
        })
        .unwrap_or_default();
    if !token_matches(auth, token) {
        return error(StatusCode::UNAUTHORIZED, "Invalid account pool token");
    }
    if request.headers().contains_key("upgrade") {
        return error(
            StatusCode::UPGRADE_REQUIRED,
            "Use HTTP streaming for account pool requests",
        );
    }
    if request.uri().scheme().is_some() {
        return error(StatusCode::BAD_REQUEST, "Use an origin-form request");
    }
    let Some((harness, mut path)) = route(request.uri().path()) else {
        return error(StatusCode::NOT_FOUND, "Unknown account pool endpoint");
    };
    accounts.refresh_pool_in_background(harness);
    if !matches!(*request.method(), hyper::Method::POST | hyper::Method::GET) {
        return error(StatusCode::METHOD_NOT_ALLOWED, "Unsupported method");
    }
    if let Some(query) = request.uri().query() {
        path.push('?');
        path.push_str(query);
    }
    let (parts, body) = request.into_parts();
    let body = match tokio::time::timeout(
        Duration::from_secs(30),
        Limited::new(body, MAX_BODY).collect(),
    )
    .await
    {
        Ok(Ok(body)) => body.to_bytes(),
        Ok(Err(_)) => {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Request body exceeds the account pool limit",
            );
        }
        Err(_) => {
            return error(
                StatusCode::REQUEST_TIMEOUT,
                "Account pool request timed out",
            );
        }
    };
    // Some Codex endpoints send compressed payloads. Opaque bodies must be
    // forwarded unchanged; metadata is optional for routing.
    let value: serde_json::Value =
        serde_json::from_slice(&body).unwrap_or_else(|_| serde_json::json!({}));
    let family = model_family(value["model"].as_str().unwrap_or_default());
    let session = session_id(harness, &parts.headers, &value);
    let mut attempted = HashSet::new();
    let mut rechecked = false;
    let mut failure = StatusCode::TOO_MANY_REQUESTS;
    loop {
        let candidate = accounts
            .pool_candidates(harness, family, session.as_deref(), &attempted, now_ms())
            .into_iter()
            .next();
        let Some(mut slot) = candidate else {
            if !rechecked {
                rechecked = true;
                // Same throttled/single-flight probes used by Settings. Keep the
                // complete target list so the shared cache retains other agents.
                accounts.refresh_pool_usage(harness, true).await;
                continue;
            }
            let mut response = error(
                failure,
                if failure == StatusCode::TOO_MANY_REQUESTS {
                    "Every pooled account is limited or unavailable"
                } else {
                    "No pooled account could complete the request"
                },
            );
            if let Some(delay) = next_unblock(&accounts, harness, family) {
                response
                    .headers_mut()
                    .insert("retry-after", delay.to_string().parse().unwrap());
            }
            return response;
        };
        attempted.insert(slot.id.clone());
        slot = match accounts.pool_secret(&slot, false).await {
            Ok(slot) => slot,
            Err(error) => {
                secret_failure(&accounts, &slot, error).await;
                continue;
            }
        };
        let mut auth_retry = false;
        let mut pace_retry = false;
        loop {
            let endpoints = lock(&accounts.inner.pool.endpoints).clone();
            let oauth = slot.credentials.get("tokens").is_some();
            let base = if harness == HarnessId::ClaudeCode {
                &endpoints.claude
            } else if oauth {
                &endpoints.codex
            } else {
                &endpoints.openai
            };
            let url = format!("{}{path}", base.trim_end_matches('/'));
            let headers = upstream_headers(harness, &parts.headers, &slot);
            let body = rewrite_body(harness, &value, &body, &slot);
            let guard = InFlight::new(&accounts, &slot.id);
            let result = accounts
                .inner
                .pool
                .http
                .request(parts.method.clone(), url)
                .headers(headers)
                .body(body)
                .send()
                .await;
            let response = match result {
                Ok(response) => response,
                Err(_) => {
                    // Never log exception messages/URLs: both can contain auth.
                    tracing::warn!(
                        provider = harness_slug(harness),
                        "account pool upstream connection failed"
                    );
                    failure = StatusCode::BAD_GATEWAY;
                    drop(guard);
                    break;
                }
            };
            let status = response.status();
            observe(&accounts, &slot, response.headers(), family);
            if status == StatusCode::UNAUTHORIZED && !auth_retry {
                auth_retry = true;
                drop(response);
                drop(guard);
                match accounts.pool_secret(&slot, true).await {
                    Ok(fresh)
                        if credentials_fingerprint(&fresh.credentials)
                            != credentials_fingerprint(&slot.credentials) =>
                    {
                        slot = fresh;
                        continue;
                    }
                    Ok(_) => {
                        mark_error(&accounts, &slot, "Needs sign-in").await;
                        failure = StatusCode::BAD_GATEWAY;
                        break;
                    }
                    Err(error) => {
                        secret_failure(&accounts, &slot, error).await;
                        failure = StatusCode::BAD_GATEWAY;
                        break;
                    }
                }
            }
            if status == StatusCode::UNAUTHORIZED {
                mark_error(&accounts, &slot, "Needs sign-in").await;
                failure = StatusCode::BAD_GATEWAY;
                break;
            }
            if status == StatusCode::FORBIDDEN {
                hold(&accounts, &slot, 30_000);
                failure = StatusCode::BAD_GATEWAY;
                break;
            }
            if status == StatusCode::TOO_MANY_REQUESTS {
                let threshold = provider(&lock(&accounts.inner.pool.config), harness)
                    .unwrap()
                    .switch_threshold;
                let quota_exhausted = {
                    let state = lock(&accounts.inner.pool.state);
                    let observation = state
                        .observations
                        .get(&slot.id)
                        .cloned()
                        .unwrap_or_default();
                    !eligible(
                        &accounts.pool_windows(&slot, &observation),
                        family,
                        threshold,
                        now_ms(),
                    )
                };
                let delay = retry_after(
                    response
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok()),
                    now_ms(),
                );
                drop(response);
                drop(guard);
                if !quota_exhausted {
                    {
                        let mut state = lock(&accounts.inner.pool.state);
                        state
                            .observations
                            .entry(slot.id.clone())
                            .or_default()
                            .held_until = Some(now_ms() + delay);
                        accounts.save_pool_state(&state);
                    }
                    if delay <= 20_000 && !pace_retry {
                        pace_retry = true;
                        tokio::time::sleep(Duration::from_millis(delay as u64)).await;
                        continue;
                    }
                }
                failure = StatusCode::TOO_MANY_REQUESTS;
                break;
            }
            if status.is_server_error() {
                failure = StatusCode::BAD_GATEWAY;
                break;
            }
            if status.is_success() {
                accounts.accept_pool_account(&slot, family, session.as_deref());
            }
            return stream_response(response, guard);
        }
    }
}

fn upstream_headers(
    harness: HarnessId,
    inbound: &reqwest::header::HeaderMap,
    slot: &Slot,
) -> reqwest::header::HeaderMap {
    use reqwest::header::{AUTHORIZATION, HeaderValue};
    let mut headers = reqwest::header::HeaderMap::new();
    for (key, value) in inbound {
        let name = key.as_str();
        let common = matches!(name, "accept" | "content-type" | "user-agent")
            || name.starts_with("x-stainless-");
        let allowed = if harness == HarnessId::ClaudeCode {
            name == "x-app" || name.starts_with("anthropic-")
        } else {
            matches!(
                name,
                "content-encoding"
                    | "openai-beta"
                    | "originator"
                    | "session-id"
                    | "session_id"
                    | "thread-id"
                    | "x-openai-internal-codex-responses-lite"
            ) || name.starts_with("x-codex-")
        };
        if common || allowed {
            headers.insert(key, value.clone());
        }
    }
    let (access, api_key) = if harness == HarnessId::ClaudeCode {
        (
            str_field(&slot.credentials["claudeAiOauth"], "accessToken"),
            str_field(&slot.credentials, "apiKey"),
        )
    } else {
        (
            str_field(&slot.credentials["tokens"], "access_token"),
            str_field(&slot.credentials, "OPENAI_API_KEY"),
        )
    };
    if harness == HarnessId::ClaudeCode && access.is_none() {
        if let Some(key) = api_key.and_then(|key| HeaderValue::from_str(&key).ok()) {
            headers.insert("x-api-key", key);
        }
    } else if let Some(token) = access
        .or(api_key)
        .and_then(|token| HeaderValue::from_str(&format!("Bearer {token}")).ok())
    {
        headers.insert(AUTHORIZATION, token);
    }
    if harness == HarnessId::ClaudeCode && slot.credentials.get("claudeAiOauth").is_some() {
        let beta = headers
            .get("anthropic-beta")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        if !beta.split(',').any(|s| s.trim() == "oauth-2025-04-20") {
            let beta = if beta.is_empty() {
                "oauth-2025-04-20".into()
            } else {
                format!("{beta},oauth-2025-04-20")
            };
            if let Ok(value) = HeaderValue::from_str(&beta) {
                headers.insert("anthropic-beta", value);
            }
        }
    }
    if harness == HarnessId::Codex {
        if let Some(id) = str_field(&slot.credentials["tokens"], "account_id")
            .and_then(|v| HeaderValue::from_str(&v).ok())
        {
            headers.insert("chatgpt-account-id", id);
        }
    }
    headers
}

pub(super) fn rewrite_body(
    harness: HarnessId,
    value: &serde_json::Value,
    original: &Bytes,
    slot: &Slot,
) -> Bytes {
    if harness != HarnessId::ClaudeCode {
        return original.clone();
    }
    let Some(account) = slot
        .claude_config
        .as_ref()
        .and_then(|c| c.pointer("/oauthAccount/accountUuid"))
        .and_then(|v| v.as_str())
    else {
        return original.clone();
    };
    let Some(user) = value.pointer("/metadata/user_id").and_then(|v| v.as_str()) else {
        return original.clone();
    };
    let rewritten = if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(user) {
        if json.get("account_uuid").is_none() {
            return original.clone();
        }
        json["account_uuid"] = account.into();
        json.to_string()
    } else if let Some(start) = user
        .strip_prefix("account_")
        .map(|_| "account_".len())
        .or_else(|| user.find("_account_").map(|at| at + "_account_".len()))
    {
        let end = user[start..]
            .find('_')
            .map(|at| start + at)
            .unwrap_or(user.len());
        format!("{}{account}{}", &user[..start], &user[end..])
    } else {
        return original.clone();
    };
    let mut value = value.clone();
    value["metadata"]["user_id"] = rewritten.into();
    Bytes::from(value.to_string())
}

fn session_id(
    harness: HarnessId,
    headers: &reqwest::header::HeaderMap,
    body: &serde_json::Value,
) -> Option<String> {
    let id = if harness == HarnessId::Codex {
        ["thread-id", "session-id", "session_id"]
            .into_iter()
            .find_map(|key| headers.get(key).and_then(|v| v.to_str().ok()))
            .or_else(|| body["prompt_cache_key"].as_str())
            .map(str::to_owned)
    } else {
        body.pointer("/metadata/user_id")
            .and_then(|v| v.as_str())
            .and_then(|user| {
                serde_json::from_str::<serde_json::Value>(user)
                    .ok()
                    .and_then(|v| str_field(&v, "session_id"))
                    .or_else(|| user.rsplit_once("_session_").map(|(_, id)| id.to_owned()))
            })
    };
    id.filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
}

pub(super) async fn mark_error(accounts: &AgentAccounts, slot: &Slot, message: &str) {
    let refresh = lock(&accounts.inner.pool.refreshes)
        .entry(slot.id.clone())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let _single = refresh.lock().await;
    let _ops = accounts.inner.ops.lock().await;
    let Some(current) = accounts.read_slot(slot.harness, &slot.id) else {
        return;
    };
    let failed = credentials_fingerprint(&slot.credentials);
    if credentials_fingerprint(&current.credentials) != failed {
        return;
    }
    let mut state = lock(&accounts.inner.pool.state);
    let observation = state.observations.entry(slot.id.clone()).or_default();
    observation.error = Some(message.into());
    observation.error_at = now_ms();
    observation.error_credentials = Some(failed);
    accounts.save_pool_state(&state);
}

fn hold(accounts: &AgentAccounts, slot: &Slot, delay: i64) {
    let mut state = lock(&accounts.inner.pool.state);
    state
        .observations
        .entry(slot.id.clone())
        .or_default()
        .held_until = Some(now_ms() + delay);
    accounts.save_pool_state(&state);
}

async fn secret_failure(accounts: &AgentAccounts, slot: &Slot, error: SecretError) {
    tracing::debug!(
        provider = harness_slug(slot.harness),
        reason = error.message(),
        "account pool credential attempt deferred"
    );
    match error {
        SecretError::Authentication(message) => mark_error(accounts, slot, message).await,
        SecretError::Temporary(_) => hold(accounts, slot, 30_000),
    }
}

fn retry_after(value: Option<&str>, now: i64) -> i64 {
    value
        .and_then(|v| {
            v.parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .map(|v| (v * 1000.0).ceil() as i64)
                .or_else(|| {
                    DateTime::parse_from_rfc2822(v)
                        .ok()
                        .map(|at| at.timestamp_millis() - now)
                })
        })
        .unwrap_or(1000)
        .clamp(1, 24 * 60 * 60 * 1000)
}

fn next_unblock(accounts: &AgentAccounts, harness: HarnessId, family: &str) -> Option<i64> {
    let now = now_ms();
    let settings = provider(&lock(&accounts.inner.pool.config), harness)?.clone();
    let state = lock(&accounts.inner.pool.state);
    accounts
        .read_slots(harness)
        .iter()
        .filter_map(|slot| {
            if settings.disabled_account_ids.contains(&slot.id) || !has_secret(slot) {
                return None;
            }
            let observation = state
                .observations
                .get(&slot.id)
                .cloned()
                .unwrap_or_default();
            if observation.error.is_some() {
                return None;
            }
            let windows = accounts.pool_windows(slot, &observation);
            let limits: Vec<_> = windows
                .iter()
                .filter(|w| {
                    applies(w, family)
                        && w.used_fraction >= settings.switch_threshold
                        && w.resets_at.is_none_or(|at| at.timestamp_millis() > now)
                })
                .collect();
            if limits.iter().any(|w| w.resets_at.is_none()) {
                return None;
            }
            observation
                .held_until
                .into_iter()
                .chain(
                    limits
                        .into_iter()
                        .filter_map(|w| w.resets_at.map(|at| at.timestamp_millis())),
                )
                .filter(|at| *at > now)
                .max()
        })
        .min()
        .map(|at| ((at - now + 999) / 1000).max(1))
}

fn number(headers: &reqwest::header::HeaderMap, key: &str) -> Option<f64> {
    headers
        .get(key)?
        .to_str()
        .ok()?
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

fn reset(headers: &reqwest::header::HeaderMap, key: &str) -> Option<DateTime<Utc>> {
    let value = headers.get(key)?.to_str().ok()?;
    if let Ok(value) = value.parse::<f64>() {
        if !value.is_finite() {
            return None;
        }
        let millis = if value < 1_000_000_000_000.0 {
            value * 1000.0
        } else {
            value
        };
        DateTime::from_timestamp_millis(millis as i64)
    } else {
        DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|at| at.with_timezone(&Utc))
    }
}

pub(super) fn observe(
    accounts: &AgentAccounts,
    slot: &Slot,
    headers: &reqwest::header::HeaderMap,
    family: &str,
) {
    let mut state = lock(&accounts.inner.pool.state);
    let observation = state.observations.entry(slot.id.clone()).or_default();
    let mut windows = accounts.pool_windows(slot, observation);
    let previous_labels: HashSet<_> = windows.iter().map(|window| window.label.clone()).collect();
    let mut changed = false;
    if slot.harness == HarnessId::ClaudeCode {
        let mut buckets = vec![
            ("5h".to_owned(), "Session".to_owned()),
            ("7d".into(), "Week".into()),
        ];
        for (key, _) in headers {
            if let Some(scope) = key
                .as_str()
                .strip_prefix("anthropic-ratelimit-unified-7d_")
                .and_then(|s| {
                    s.strip_suffix("-utilization")
                        .or_else(|| s.strip_suffix("-status"))
                        .or_else(|| s.strip_suffix("-reset"))
                })
            {
                // Scoped buckets are model-specific, even when the provider's
                // bucket name is opaque: attribute it to the request's model.
                let known = model_family(scope);
                let family = if known == "other" { family } else { known };
                if family != "other" {
                    let bucket = (format!("7d_{scope}"), format!("Week ({family})"));
                    if !buckets.contains(&bucket) {
                        buckets.push(bucket);
                    }
                }
            }
        }
        for (bucket, label) in buckets {
            let prefix = format!("anthropic-ratelimit-unified-{bucket}");
            let used = number(headers, &format!("{prefix}-utilization"));
            let resets_at = reset(headers, &format!("{prefix}-reset")).or_else(|| {
                observation
                    .quota_resets
                    .get(&label)
                    .copied()
                    .filter(|at| at.timestamp_millis() > now_ms())
            });
            let rejected = headers
                .get(format!("{prefix}-status"))
                .is_some_and(|s| s == "rejected");
            if let Some(reset) = resets_at {
                changed |= observation.quota_resets.insert(label.clone(), reset) != Some(reset);
            }
            changed |= update_window(
                &mut windows,
                label,
                used.map(|u| u as f32),
                resets_at,
                rejected,
            );
        }
    } else {
        for bucket in ["primary", "secondary"] {
            let prefix = format!("x-codex-{bucket}");
            if !headers
                .keys()
                .any(|key| key.as_str().starts_with(&format!("{prefix}-")))
            {
                continue;
            }
            let minutes = number(headers, &format!("{prefix}-window-minutes"));
            let label = minutes
                .filter(|minutes| *minutes > 0.0)
                .map(|m| codex_window_label((m * 60.0) as i64))
                .map(str::to_owned)
                .or_else(|| observation.codex_window_labels.get(bucket).cloned())
                .unwrap_or_else(|| {
                    if bucket == "primary" {
                        "Primary"
                    } else {
                        "Secondary"
                    }
                    .into()
                });
            if let Some(previous) = observation
                .codex_window_labels
                .insert(bucket.into(), label.clone())
            {
                if previous != label {
                    windows.retain(|window| window.label != previous);
                    observation.replaced_window_labels.insert(previous);
                    observation.quota_resets.remove(&format!("codex:{bucket}"));
                    changed = true;
                }
            } else {
                changed = true;
            }
            observation.replaced_window_labels.remove(&label);
            let resets_at = reset(headers, &format!("{prefix}-reset-at")).or_else(|| {
                number(headers, &format!("{prefix}-reset-after-seconds")).and_then(|s| {
                    DateTime::from_timestamp_millis(now_ms().saturating_add((s * 1000.0) as i64))
                })
            });
            let used =
                number(headers, &format!("{prefix}-used-percent")).map(|u| (u / 100.0) as f32);
            let resets_at = resets_at.or_else(|| {
                observation
                    .quota_resets
                    .get(&format!("codex:{bucket}"))
                    .copied()
                    .filter(|at| at.timestamp_millis() > now_ms())
            });
            if let Some(reset) = resets_at {
                changed |= observation
                    .quota_resets
                    .insert(format!("codex:{bucket}"), reset)
                    != Some(reset);
            }
            let rejected = headers
                .get(format!("{prefix}-over-limit"))
                .is_some_and(|v| v == "true");
            let mut own_window: Vec<_> = observation
                .codex_windows
                .get(bucket)
                .filter(|window| window.label == label)
                .cloned()
                .into_iter()
                .collect();
            changed |= update_window(&mut own_window, label, used, resets_at, rejected);
            match own_window.pop() {
                Some(window) => {
                    observation.codex_windows.insert(bucket.into(), window);
                }
                None => {
                    observation.codex_windows.remove(bucket);
                }
            }
        }
        windows.retain(|window| {
            !observation.replaced_window_labels.contains(&window.label)
                && !observation
                    .codex_window_labels
                    .values()
                    .any(|label| *label == window.label)
        });
        for bucket in ["primary", "secondary"] {
            if let Some(window) = observation.codex_windows.get(bucket) {
                windows.push(window.clone());
            }
        }
    }
    if changed {
        let current_labels: HashSet<_> =
            windows.iter().map(|window| window.label.clone()).collect();
        observation
            .replaced_window_labels
            .extend(previous_labels.difference(&current_labels).cloned());
        observation
            .replaced_window_labels
            .retain(|label| !current_labels.contains(label));
        observation.windows = windows;
        observation.observed_at = now_ms();
        accounts.persist_pool_state(&state, false);
    }
}

fn update_window(
    windows: &mut Vec<AgentUsageWindow>,
    label: String,
    used: Option<f32>,
    reset: Option<DateTime<Utc>>,
    rejected: bool,
) -> bool {
    if used.is_none() && reset.is_none() && !rejected {
        return false;
    }
    let previous = windows.iter().find(|w| w.label == label).cloned();
    if used.is_none()
        && !rejected
        && previous.as_ref().is_some_and(|window| {
            window
                .resets_at
                .is_some_and(|at| at.timestamp_millis() <= now_ms())
        })
        && reset.is_some_and(|at| at.timestamp_millis() > now_ms())
    {
        // The old period ended. A new reset date says nothing about use in
        // the replacement period; don't carry an exhausted reading across it.
        windows.retain(|window| window.label != label);
        return true;
    }
    if used.is_none() && !rejected && previous.is_none() {
        // A reset date alone is not a measurement of consumed quota.
        return false;
    }
    windows.retain(|w| w.label != label);
    windows.push(AgentUsageWindow {
        label,
        used_fraction: if rejected {
            1.0
        } else {
            used.or_else(|| previous.as_ref().map(|w| w.used_fraction))
                .unwrap_or(0.0)
                .clamp(0.0, 1.0)
        },
        resets_at: reset.or_else(|| {
            previous
                .and_then(|w| w.resets_at)
                .filter(|at| at.timestamp_millis() > now_ms())
        }),
    });
    true
}

struct InFlight {
    inner: Weak<Inner>,
    id: String,
}
impl InFlight {
    fn new(accounts: &AgentAccounts, id: &str) -> Self {
        lock(&accounts.inner.pool.state)
            .observations
            .entry(id.into())
            .or_default()
            .in_flight += 1;
        Self {
            inner: Arc::downgrade(&accounts.inner),
            id: id.into(),
        }
    }
}
impl Drop for InFlight {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            let mut state = lock(&inner.pool.state);
            if let Some(observation) = state.observations.get_mut(&self.id) {
                observation.in_flight = observation.in_flight.saturating_sub(1);
            }
        }
    }
}

fn stream_response(response: reqwest::Response, guard: InFlight) -> Response<Body> {
    let status = response.status();
    let mut headers = response.headers().clone();
    let sse = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream"));
    // Hop-by-hop headers named by Connection are also stripped.
    let connection: Vec<_> = headers
        .get("connection")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_owned())
        .collect();
    for key in connection.iter().map(String::as_str).chain([
        "connection",
        "keep-alive",
        "transfer-encoding",
        "te",
        "trailer",
        "upgrade",
        "proxy-authenticate",
        "proxy-authorization",
    ]) {
        headers.remove(key);
    }
    let stream = futures::stream::unfold(
        (response.bytes_stream(), Some(guard), false),
        move |(mut stream, mut guard, ended)| async move {
            if ended {
                return None;
            }
            match stream.next().await {
                Some(Ok(bytes)) => Some((
                    Ok::<_, Box<dyn Error + Send + Sync>>(Frame::data(bytes)),
                    (stream, guard, false),
                )),
                Some(Err(error)) => {
                    guard.take();
                    if sse {
                        let frame = Bytes::from_static(b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"api_error\",\"message\":\"Account pool upstream stream interrupted\"}}\n\n");
                        Some((Ok(Frame::data(frame)), (stream, guard, true)))
                    } else {
                        Some((
                            Err(Box::new(error) as Box<dyn Error + Send + Sync>),
                            (stream, guard, true),
                        ))
                    }
                }
                None => None,
            }
        },
    );
    let mut result = Response::new(StreamBody::new(stream).boxed_unsync());
    *result.status_mut() = status;
    *result.headers_mut() = headers;
    // The SSE error frame may change the final length.
    if sse {
        result.headers_mut().remove("content-length");
    }
    result
}
