//! Authenticated Streamable HTTP with per-client MCP session state.
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use tokio::{net::TcpListener, sync::Mutex};

use crate::{Tools, jsonrpc::handle_request};

#[derive(Clone)]
pub struct HttpConfig {
    pub bind: SocketAddr,
    /// Never included in Debug or logs. Use a random token of at least 32 bytes.
    pub token: String,
    pub sender_label: Option<String>,
}

impl HttpConfig {
    /// No bind means disabled in the daemon. The CLI supplies a loopback default.
    pub fn from_env(default_bind: Option<&str>) -> anyhow::Result<Option<Self>> {
        let bind = std::env::var("ZERON_MCP_HTTP_BIND")
            .ok()
            .or_else(|| default_bind.map(str::to_owned));
        let Some(bind) = bind else {
            return Ok(None);
        };
        let config = Self {
            bind: bind
                .parse()
                .context("ZERON_MCP_HTTP_BIND must be an IP:port")?,
            token: std::env::var("ZERON_MCP_HTTP_TOKEN")
                .context("ZERON_MCP_HTTP_TOKEN is required for HTTP MCP")?,
            sender_label: std::env::var("ZERON_MCP_SENDER_LABEL").ok(),
        };
        config.validate()?;
        Ok(Some(config))
    }

    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.bind.ip().is_unspecified() && !self.bind.ip().is_multicast(),
            "bind MCP HTTP to a specific loopback, tailnet or LAN address"
        );
        anyhow::ensure!(
            self.token.len() >= 32 && self.token.bytes().all(|b| b.is_ascii_graphic()),
            "MCP HTTP token must contain at least 32 printable non-space ASCII bytes"
        );
        Ok(())
    }
}

struct Session {
    tools: Arc<Tools>,
    touched: Instant,
    cancellation: tokio_util::sync::CancellationToken,
    active: Arc<StdMutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        // Deletion and expiry end the session's waits, including their RPC watches.
        self.cancellation.cancel();
    }
}

struct ActiveRequest {
    id: String,
    active: Arc<StdMutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
}

impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.id);
    }
}

struct Server {
    token: String,
    factory: Arc<dyn Fn() -> anyhow::Result<Tools> + Send + Sync>,
    sessions: Mutex<HashMap<String, Session>>,
    requests: tokio::sync::Semaphore,
    shutdown: tokio_util::sync::CancellationToken,
}

/// The listener is supplied separately so callers can bind before reporting readiness.
pub async fn serve_http(
    listener: TcpListener,
    config: HttpConfig,
    factory: Arc<dyn Fn() -> anyhow::Result<Tools> + Send + Sync>,
) -> anyhow::Result<()> {
    config.validate()?;
    // Validate sender configuration before accepting any client.
    drop(factory()?);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let _guard = shutdown.clone().drop_guard();
    let state = Arc::new(Server {
        shutdown: shutdown.clone(),
        token: config.token,
        factory,
        sessions: Mutex::new(HashMap::new()),
        requests: tokio::sync::Semaphore::new(64),
    });
    let app = Router::new()
        .route("/mcp", any(endpoint))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(state);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await
        .context("serving MCP HTTP")
}

async fn endpoint(State(server): State<Arc<Server>>, request: axum::extract::Request) -> Response {
    tokio::select! {
        response = dispatch(&server, request) => response,
        _ = server.shutdown.cancelled() => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn dispatch(server: &Server, request: axum::extract::Request) -> Response {
    let headers = request.headers();
    let expected = format!("Bearer {}", server.token);
    let authenticated = headers
        .get(header::AUTHORIZATION)
        .is_some_and(|h| bool::from(h.as_bytes().ct_eq(expected.as_bytes())));
    if !authenticated {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response();
    }
    // This endpoint is for agents. No browser origins or credentialed CORS.
    if headers.contains_key(header::ORIGIN) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(version) = headers.get("mcp-protocol-version") {
        if !matches!(version.to_str(), Ok("2025-06-18" | "2025-03-26")) {
            return StatusCode::BAD_REQUEST.into_response();
        }
    }
    if request.method() == axum::http::Method::GET {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "POST, DELETE")],
        )
            .into_response();
    }
    let session_id = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if request.method() == axum::http::Method::DELETE {
        return match session_id {
            Some(id) if server.sessions.lock().await.remove(&id).is_some() => {
                StatusCode::NO_CONTENT.into_response()
            }
            Some(_) => StatusCode::NOT_FOUND.into_response(),
            None => StatusCode::BAD_REQUEST.into_response(),
        };
    }
    if request.method() != axum::http::Method::POST {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    if !headers
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|v| v.split(';').next().unwrap_or("").trim() == "application/json")
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    if !accepts(headers, "application/json") || !accepts(headers, "text/event-stream") {
        return StatusCode::NOT_ACCEPTABLE.into_response();
    }
    // Control notifications must still be accepted when all tool slots are
    // occupied, so parse before reserving a slot. Bound slow body uploads too.
    let bytes = match tokio::time::timeout(
        Duration::from_secs(10),
        axum::body::to_bytes(request.into_body(), 1024 * 1024),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
    };
    let message: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error"}}),
            ),
        )
            .into_response(),
    };
    if !message.is_object() || message["jsonrpc"] != "2.0" {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let method = message.get("method").and_then(Value::as_str);
    let id = message.get("id").cloned();
    if method.is_none()
        && !(id.is_some() && (message.get("result").is_some() || message.get("error").is_some()))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if id
        .as_ref()
        .is_some_and(|id| !id.is_string() && !id.is_number())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let _permit = if id.is_some() && method.is_some() {
        match server.requests.try_acquire() {
            Ok(permit) => Some(permit),
            Err(_) => return StatusCode::TOO_MANY_REQUESTS.into_response(),
        }
    } else {
        None
    };
    let initializing = method == Some("initialize") && id.is_some();
    let (tools, session_id, cancellation, active) = {
        let mut sessions = server.sessions.lock().await;
        sessions.retain(|_, s| s.touched.elapsed() < Duration::from_secs(86400));
        if initializing && session_id.is_none() {
            if sessions.len() >= 128 {
                return StatusCode::TOO_MANY_REQUESTS.into_response();
            }
            let tools = match (server.factory)() {
                Ok(t) => Arc::new(t),
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            };
            let id = uuid::Uuid::new_v4().to_string();
            let cancellation = tokio_util::sync::CancellationToken::new();
            let active = Arc::new(StdMutex::new(HashMap::new()));
            sessions.insert(
                id.clone(),
                Session {
                    tools: tools.clone(),
                    touched: Instant::now(),
                    cancellation: cancellation.clone(),
                    active: active.clone(),
                },
            );
            (tools, id, cancellation, active)
        } else {
            let Some(id) = session_id else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            let Some(session) = sessions.get_mut(&id) else {
                return StatusCode::NOT_FOUND.into_response();
            };
            session.touched = Instant::now();
            (
                session.tools.clone(),
                id,
                session.cancellation.clone(),
                session.active.clone(),
            )
        }
    };
    let Some(id) = id else {
        if method == Some("notifications/cancelled") {
            // Like unknown/finished IDs, malformed cancellation notices are
            // ignored: they are fire-and-forget protocol notifications.
            if let Some(request_id) = message
                .get("params")
                .and_then(|p| p.get("requestId"))
                .filter(|id| id.is_string() || id.is_number())
                && let Some(request) = active
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(&request_id.to_string())
            {
                request.cancel();
            }
        }
        return StatusCode::ACCEPTED.into_response();
    };
    let Some(method) = method else {
        return StatusCode::ACCEPTED.into_response();
    };
    let mut params = message.get("params").cloned().unwrap_or(Value::Null);
    // The stdio dispatcher also supports the older stdio-only revision.
    // HTTP must negotiate a revision this transport can accept afterwards.
    if initializing
        && params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .is_some_and(|v| !matches!(v, "2025-06-18" | "2025-03-26"))
    {
        params["protocolVersion"] = json!("2025-06-18");
    }
    let request_cancellation = cancellation.child_token();
    let _request = if !initializing {
        let key = id.to_string();
        let mut requests = active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if requests.contains_key(&key) {
            return StatusCode::BAD_REQUEST.into_response();
        }
        requests.insert(key.clone(), request_cancellation.clone());
        Some(ActiveRequest {
            id: key,
            active: active.clone(),
        })
    } else {
        None
    };
    let response = tokio::select! {
        biased;
        _ = request_cancellation.cancelled() => {
            if cancellation.is_cancelled() { return StatusCode::NOT_FOUND.into_response(); }
            // End the HTTP exchange without a JSON-RPC response to the cancelled request.
            return StatusCode::NO_CONTENT.into_response();
        }
        response = handle_request(&tools, id.clone(), method, params) => response,
    };
    ([("mcp-session-id", session_id)], Json(response)).into_response()
}

fn accepts(headers: &HeaderMap, mime: &str) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|v| v.split(';').next().unwrap_or("").trim() == mime)
}

/// Standalone proxy, useful when the engine is already running.
pub async fn run(config: crate::McpConfig, http: HttpConfig) -> anyhow::Result<()> {
    let listener = TcpListener::bind(http.bind)
        .await
        .context("binding MCP HTTP")?;
    let label = http.sender_label.clone();
    let factory = Arc::new(move || {
        Tools::new(Arc::new(crate::Zeron::new(
            format!("ws://127.0.0.1:{}", config.ipc_port),
            config.origin.clone(),
        )))
        .with_sender_label(label.clone())
    });
    serve_http(listener, http, factory).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    const TOKEN: &str = "test-only-controller-token-1234567890";

    fn post(client: &Client, url: &str) -> reqwest::RequestBuilder {
        client
            .post(url)
            .bearer_auth(TOKEN)
            .header("Accept", "application/json, text/event-stream")
    }

    #[tokio::test]
    async fn http_auth_origins_sessions_and_notifications() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config = HttpConfig {
            bind: addr,
            token: TOKEN.into(),
            sender_label: None,
        };
        let factory = Arc::new(|| {
            Ok(Tools::new(Arc::new(crate::Zeron::new(
                "ws://127.0.0.1:1".into(),
                crate::Origin::default(),
            ))))
        });
        let server = tokio::spawn(serve_http(listener, config, factory));
        let url = format!("http://{addr}/mcp");
        let client = Client::new();
        assert_eq!(client.post(&url).send().await.unwrap().status(), 401);
        assert_eq!(
            client
                .get(&url)
                .bearer_auth("wrong")
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            post(&client, &url)
                .header("Origin", "https://example.test")
                .json(&json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        assert_eq!(
            client
                .get(&url)
                .bearer_auth(TOKEN)
                .send()
                .await
                .unwrap()
                .status(),
            405
        );
        assert_eq!(
            post(&client, &url)
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}});
        let response = post(&client, &url).json(&init).send().await.unwrap();
        assert_eq!(response.status(), 200);
        let session = response.headers()["mcp-session-id"]
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            response.json::<Value>().await.unwrap()["result"]["protocolVersion"],
            "2025-06-18"
        );
        let other = post(&client, &url).json(&init).send().await.unwrap();
        assert_ne!(other.headers()["mcp-session-id"], session);
        let legacy = post(&client, &url).json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}})).send().await.unwrap();
        assert_eq!(
            legacy.json::<Value>().await.unwrap()["result"]["protocolVersion"],
            "2025-06-18"
        );
        let response = post(&client, &url)
            .header("mcp-session-id", &session)
            .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
            .send()
            .await
            .unwrap();
        let body = response.json::<Value>().await.unwrap();
        assert!(
            body["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["name"] == "wait_for_any")
        );
        let notification = post(&client, &url)
            .header("mcp-session-id", &session)
            .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .send()
            .await
            .unwrap();
        assert_eq!(notification.status(), 202);
        assert!(notification.bytes().await.unwrap().is_empty());
        assert_eq!(
            post(&client, &url)
                .header("mcp-session-id", &session)
                .header("mcp-protocol-version", "bad")
                .json(&init)
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        assert_eq!(
            client
                .delete(&url)
                .bearer_auth(TOKEN)
                .header("mcp-session-id", &session)
                .send()
                .await
                .unwrap()
                .status(),
            204
        );
        assert_eq!(
            post(&client, &url)
                .header("mcp-session-id", &session)
                .json(&init)
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        server.abort();
    }

    #[test]
    fn refuses_wildcard_and_weak_auth() {
        let mut config = HttpConfig {
            bind: "0.0.0.0:1234".parse().unwrap(),
            token: TOKEN.into(),
            sender_label: None,
        };
        assert!(config.validate().is_err());
        config.bind = "100.100.100.100:1234".parse().unwrap();
        assert!(config.validate().is_ok());
        config.token = "short".into();
        assert!(config.validate().is_err());
    }
    fn blocking_server() -> (Arc<Server>, tokio::sync::watch::Sender<()>) {
        use futures::StreamExt;
        struct Service(tokio::sync::watch::Sender<()>);
        #[async_trait::async_trait]
        impl zeron_rpc::RpcService for Service {
            async fn handle(
                &self,
                method: &str,
                _: Value,
            ) -> Result<zeron_rpc::RpcReply, zeron_rpc::RpcError> {
                match method {
                    zeron_rpc::methods::WATCH_CHATS => Ok(zeron_rpc::RpcReply::Stream(futures::stream::once(async {
                        json!([{"id":"busy","deviceId":"device","archived":false,"createdAt":"2026-10-15T00:00:00Z"}])
                    }).boxed())),
                    zeron_rpc::methods::WATCH_SESSIONS => {
                        let rx = self.0.subscribe();
                        Ok(zeron_rpc::RpcReply::Stream(futures::stream::unfold((true, rx), |(first, mut rx)| async move {
                            if !first { rx.changed().await.ok()?; }
                            Some((json!([{"chatId":"busy","deviceId":"device","status":"working","updatedAt":chrono::Utc::now()}]), (false, rx)))
                        }).boxed()))
                    }
                    _ => Err(zeron_rpc::RpcError::UnknownMethod(method.into())),
                }
            }
        }
        let (watched, _) = tokio::sync::watch::channel(());
        let service = Arc::new(Service(watched.clone()));
        let factory = Arc::new(move || {
            Ok(Tools::new(Arc::new(crate::Zeron::with_client(
                zeron_rpc::memory_client(service.clone()),
                crate::Origin::default(),
            ))))
        });
        (
            Arc::new(Server {
                token: TOKEN.into(),
                factory,
                sessions: Mutex::new(HashMap::new()),
                requests: tokio::sync::Semaphore::new(64),
                shutdown: tokio_util::sync::CancellationToken::new(),
            }),
            watched,
        )
    }

    async fn post_rpc(server: &Server, session: Option<&str>, message: Value) -> Response {
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri("/mcp")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream");
        if let Some(session) = session {
            request = request.header("mcp-session-id", session);
        }
        dispatch(
            server,
            request
                .body(axum::body::Body::from(message.to_string()))
                .unwrap(),
        )
        .await
    }

    async fn initialize_session(server: &Server) -> String {
        let response = post_rpc(
            server,
            None,
            json!({"jsonrpc":"2.0","id":0,"method":"initialize"}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        response.headers()["mcp-session-id"]
            .to_str()
            .unwrap()
            .to_owned()
    }

    fn wait_request(
        server: Arc<Server>,
        session: String,
        id: Value,
    ) -> tokio::task::JoinHandle<Response> {
        tokio::spawn(async move {
            post_rpc(&server, Some(&session), json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"wait_for_turn","arguments":{"chat":"busy","timeout_secs":3600}}})).await
        })
    }

    async fn wait_subscriptions(watched: &tokio::sync::watch::Sender<()>, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while watched.receiver_count() != count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("active wait subscription count");
    }

    #[tokio::test]
    async fn cancellation_is_scoped_to_session_and_releases_watches() {
        let (server, watched) = blocking_server();
        let first = initialize_session(&server).await;
        let second = initialize_session(&server).await;
        let first_wait = wait_request(server.clone(), first.clone(), json!("same-id"));
        let second_wait = wait_request(server.clone(), second.clone(), json!("same-id"));
        wait_subscriptions(&watched, 2).await;
        let cancelled = post_rpc(&server, Some(&first), json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"same-id"}})).await;
        assert_eq!(cancelled.status(), StatusCode::ACCEPTED);
        let response = tokio::time::timeout(Duration::from_secs(1), first_wait)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .is_empty()
        );
        wait_subscriptions(&watched, 1).await;
        assert!(
            !second_wait.is_finished(),
            "another session's request must keep waiting"
        );
        let request = axum::http::Request::builder()
            .method("DELETE")
            .uri("/mcp")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header("mcp-session-id", &second)
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            dispatch(&server, request).await.status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), second_wait)
                .await
                .unwrap()
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        wait_subscriptions(&watched, 0).await;
        assert_eq!(server.requests.available_permits(), 64);
    }

    #[tokio::test]
    async fn deletion_frees_a_saturated_server_and_cancellation_bypasses_the_limit() {
        let (server, watched) = blocking_server();
        let session = initialize_session(&server).await;
        let waits = (1..=64)
            .map(|id| wait_request(server.clone(), session.clone(), json!(id)))
            .collect::<Vec<_>>();
        wait_subscriptions(&watched, 64).await;
        assert_eq!(server.requests.available_permits(), 0);
        let notification = post_rpc(
            &server,
            Some(&session),
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}),
        )
        .await;
        assert_eq!(notification.status(), StatusCode::ACCEPTED);
        wait_subscriptions(&watched, 63).await;
        let request = axum::http::Request::builder()
            .method("DELETE")
            .uri("/mcp")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header("mcp-session-id", &session)
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            dispatch(&server, request).await.status(),
            StatusCode::NO_CONTENT
        );
        for wait in waits {
            tokio::time::timeout(Duration::from_secs(1), wait)
                .await
                .unwrap()
                .unwrap();
        }
        wait_subscriptions(&watched, 0).await;
        assert_eq!(server.requests.available_permits(), 64);
        initialize_session(&server).await;
    }

    #[tokio::test]
    async fn expiry_cancels_active_requests_and_duplicate_ids_are_rejected() {
        let (server, watched) = blocking_server();
        let session = initialize_session(&server).await;
        let wait = wait_request(server.clone(), session.clone(), json!(1));
        wait_subscriptions(&watched, 1).await;
        let duplicate = post_rpc(
            &server,
            Some(&session),
            json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
        )
        .await;
        assert_eq!(duplicate.status(), StatusCode::BAD_REQUEST);
        server
            .sessions
            .lock()
            .await
            .get_mut(&session)
            .unwrap()
            .touched = Instant::now() - Duration::from_secs(86401);
        initialize_session(&server).await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), wait)
                .await
                .unwrap()
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        wait_subscriptions(&watched, 0).await;
        assert_eq!(server.requests.available_permits(), 64);
    }
}
