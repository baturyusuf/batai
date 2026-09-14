//! Loopback HTTP transport for the shared Rust application kernel.

use std::{future::Future, net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    application::{BataiApplication, DecisionRequest, LegacyAssignTask, LegacyCreateAgent},
    runtime::errors::RuntimeError,
};

pub const DEFAULT_PORT: u16 = 4317;
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct HttpState {
    application: Arc<BataiApplication>,
    control_token: Arc<str>,
    allow_legacy_unauthenticated_mutations: bool,
    runtime_status: Arc<dyn Fn() -> String + Send + Sync>,
}

impl HttpState {
    pub fn new(
        application: Arc<BataiApplication>,
        control_token: String,
        allow_legacy_unauthenticated_mutations: bool,
    ) -> Self {
        Self {
            application,
            control_token: control_token.into(),
            allow_legacy_unauthenticated_mutations,
            runtime_status: Arc::new(|| "READY".into()),
        }
    }

    pub fn with_runtime_status_provider(
        mut self,
        provider: Arc<dyn Fn() -> String + Send + Sync>,
    ) -> Self {
        self.runtime_status = provider;
        self
    }

    pub fn generated(application: Arc<BataiApplication>) -> Self {
        let token = std::env::var("BATAI_CONTROL_TOKEN")
            .unwrap_or_else(|_| uuid::Uuid::new_v4().simple().to_string());
        let legacy = std::env::var("BATAI_LEGACY_HTTP_MUTATIONS")
            .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
        Self::new(application, token, legacy)
    }

    pub fn control_token(&self) -> &str {
        &self.control_token
    }
}

#[derive(RustEmbed)]
#[folder = "../src/ui"]
struct UiAssets;

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
    details: Value,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    details: Value,
}

impl ApiError {
    fn validation(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "VALIDATION_ERROR",
            message: message.into(),
            details: json!({}),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "FORBIDDEN",
            message: message.into(),
            details: json!({}),
        }
    }

    fn from_runtime(error: RuntimeError) -> Self {
        match error {
            RuntimeError::OrganizationConflict { expected, current } => Self {
                status: StatusCode::CONFLICT,
                code: "REVISION_CONFLICT",
                message: "The organization changed before this request was applied".into(),
                details: json!({"expected":expected,"current":current}),
            },
            RuntimeError::AgentNotFound(_) | RuntimeError::TaskNotFound(_) => Self {
                status: StatusCode::NOT_FOUND,
                code: "NOT_FOUND",
                message: error.to_string(),
                details: json!({}),
            },
            RuntimeError::Governance(message) => {
                let lowered = message.to_ascii_lowercase();
                let status = if lowered.contains("denied")
                    || lowered.contains("authority")
                    || lowered.contains("permission")
                {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::BAD_REQUEST
                };
                Self {
                    status,
                    code: if status == StatusCode::FORBIDDEN {
                        "GOVERNANCE_DENIED"
                    } else {
                        "VALIDATION_ERROR"
                    },
                    message,
                    details: json!({}),
                }
            }
            _ => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "INTERNAL_ERROR",
                message: "The Batai control plane could not complete the request".into(),
                details: json!({}),
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.code,
                message: self.message,
                details: self.details,
            }),
        )
            .into_response()
    }
}

pub fn router(state: HttpState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/state", get(state_snapshot))
        .route("/api/providers", get(provider_connections))
        .route("/api/agents", post(create_agent))
        .route("/api/tasks", post(create_task))
        .route("/api/god/messages", post(send_god_message))
        .route("/api/decisions", post(create_decision))
        .route("/api/decisions/{id}/resolve", post(resolve_decision))
        .route("/api/tasks/{id}/approve", post(approve_task))
        .route("/api/{*path}", get(api_not_found).post(api_not_found))
        .route("/", get(index))
        .route("/{*path}", get(static_asset))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn(request_guard))
        .with_state(state)
}

pub async fn serve<F>(state: HttpState, address: SocketAddr, shutdown: F) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    if !address.ip().is_loopback() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Batai HTTP control plane only binds to loopback",
        ));
    }
    let listener = tokio::net::TcpListener::bind(address).await?;
    serve_listener(state, listener, shutdown).await
}

pub async fn serve_listener<F>(
    state: HttpState,
    listener: tokio::net::TcpListener,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    if !listener.local_addr()?.ip().is_loopback() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Batai HTTP control plane only serves a loopback listener",
        ));
    }
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}

async fn request_guard(request: Request, next: Next) -> Result<Response, ApiError> {
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::forbidden("A valid loopback Host header is required"))?;
    if !valid_loopback_authority(host) {
        return Err(ApiError::forbidden(
            "Host is not an approved loopback authority",
        ));
    }
    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        if !valid_loopback_origin(origin) {
            return Err(ApiError::forbidden(
                "Cross-origin local API access is not allowed",
            ));
        }
    }
    if request.method() == Method::OPTIONS {
        return Err(ApiError::forbidden("Cross-origin preflight is not enabled"));
    }
    if matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH
    ) && headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| !value.to_ascii_lowercase().starts_with("application/json"))
    {
        return Err(ApiError::validation(
            "Content-Type must be application/json",
        ));
    }
    let response = next.run(request).await;
    if response.status() == StatusCode::METHOD_NOT_ALLOWED {
        return Err(ApiError {
            status: StatusCode::METHOD_NOT_ALLOWED,
            code: "METHOD_NOT_ALLOWED",
            message: "The requested method is not supported for this endpoint".into(),
            details: json!({}),
        });
    }
    Ok(response)
}

fn valid_loopback_authority(authority: &str) -> bool {
    let host = authority
        .rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|character| character.is_ascii_digit()))
        .map_or(authority, |(host, _)| host);
    matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "[::1]"
    )
}

fn valid_loopback_origin(origin: &str) -> bool {
    let Some(authority) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    valid_loopback_authority(authority.trim_end_matches('/'))
}

fn authorize(headers: &HeaderMap, state: &HttpState) -> Result<(), ApiError> {
    if state.allow_legacy_unauthenticated_mutations {
        return Ok(());
    }
    let expected = format!("Bearer {}", state.control_token);
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if supplied == Some(expected.as_str()) {
        Ok(())
    } else {
        Err(ApiError::forbidden(
            "A valid Batai local control token is required for mutations",
        ))
    }
}

fn json_rejection(error: axum::extract::rejection::JsonRejection) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            code: "REQUEST_TOO_LARGE",
            message: "Request body exceeds the 1 MiB control-plane limit".into(),
            details: json!({"maxBytes":MAX_REQUEST_BYTES}),
        }
    } else {
        ApiError::validation(error.body_text())
    }
}

async fn health(State(state): State<HttpState>) -> Json<Value> {
    let runtime = (state.runtime_status)();
    Json(json!({
        "ok": true,
        "status": "ok",
        "runtime": runtime,
        "runtimeEngine": "rust",
        "owner": "daemon",
        "controlPlane": "batai"
    }))
}

async fn state_snapshot(State(state): State<HttpState>) -> Result<Json<Value>, ApiError> {
    state
        .application
        .external_snapshot()
        .map(Json)
        .map_err(ApiError::from_runtime)
}

async fn provider_connections(State(state): State<HttpState>) -> Json<Value> {
    Json(Value::Array(
        state
            .application
            .providers()
            .into_iter()
            .map(|provider| {
                let name = match provider.id.as_str() {
                    "codex" => "codex-cli",
                    "claude" => "claude-cli",
                    other => other,
                };
                let status = if provider.status.eq_ignore_ascii_case("connected") {
                    "AVAILABLE"
                } else {
                    "OFFLINE"
                };
                json!({
                    "name":name,
                    "usage":{"status":status},
                    "models":[],
                    "connection":provider,
                })
            })
            .collect(),
    ))
}

async fn create_agent(
    State(state): State<HttpState>,
    headers: HeaderMap,
    payload: Result<Json<LegacyCreateAgent>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&headers, &state)?;
    let Json(input) = payload.map_err(json_rejection)?;
    let result = state
        .application
        .create_agent(input)
        .map_err(ApiError::from_runtime)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(result).unwrap_or(Value::Null)),
    ))
}

async fn create_task(
    State(state): State<HttpState>,
    headers: HeaderMap,
    payload: Result<Json<LegacyAssignTask>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&headers, &state)?;
    let Json(input) = payload.map_err(json_rejection)?;
    let result = state
        .application
        .assign_task(input)
        .await
        .map_err(ApiError::from_runtime)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(result).unwrap_or(Value::Null)),
    ))
}

#[derive(Deserialize)]
struct GodMessageRequest {
    content: String,
}

async fn send_god_message(
    State(state): State<HttpState>,
    headers: HeaderMap,
    payload: Result<Json<GodMessageRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&headers, &state)?;
    let Json(input) = payload.map_err(json_rejection)?;
    let result = state
        .application
        .send_god_message(&input.content)
        .map_err(ApiError::from_runtime)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(result).unwrap_or(Value::Null)),
    ))
}

async fn create_decision(
    State(state): State<HttpState>,
    headers: HeaderMap,
    payload: Result<Json<DecisionRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&headers, &state)?;
    let Json(input) = payload.map_err(json_rejection)?;
    let result = state
        .application
        .request_god_decision(input)
        .map_err(ApiError::from_runtime)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(result).unwrap_or(Value::Null)),
    ))
}

#[derive(Deserialize)]
struct ResolveDecisionRequest {
    value: Value,
    #[serde(default)]
    note: Option<String>,
}

async fn resolve_decision(
    State(state): State<HttpState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<ResolveDecisionRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    authorize(&headers, &state)?;
    let Json(input) = payload.map_err(json_rejection)?;
    let approve = match input.value {
        Value::Bool(value) => value,
        Value::String(value) => matches!(
            value.to_ascii_uppercase().as_str(),
            "APPROVE" | "APPROVED" | "ALLOW" | "YES"
        ),
        _ => {
            return Err(ApiError::validation(
                "decision value must be boolean or a decision word",
            ))
        }
    };
    let result = state
        .application
        .resolve_decision(&id, approve, input.note)
        .map_err(ApiError::from_runtime)?;
    Ok(Json(serde_json::to_value(result).unwrap_or(Value::Null)))
}

async fn approve_task(
    State(state): State<HttpState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    authorize(&headers, &state)?;
    let result = state
        .application
        .approve_task(&id)
        .await
        .map_err(ApiError::from_runtime)?;
    Ok(Json(serde_json::to_value(result).unwrap_or(Value::Null)))
}

async fn api_not_found() -> ApiError {
    ApiError {
        status: StatusCode::NOT_FOUND,
        code: "NOT_FOUND",
        message: "API route not found".into(),
        details: json!({}),
    }
}

async fn index() -> Response {
    embedded("index.html")
}

async fn static_asset(Path(path): Path<String>) -> Response {
    if path.split('/').any(|component| component == "..") {
        return ApiError::forbidden("Invalid static asset path").into_response();
    }
    embedded(path.trim_start_matches('/'))
}

fn embedded(path: &str) -> Response {
    let Some(asset) = UiAssets::get(path) else {
        return ApiError {
            status: StatusCode::NOT_FOUND,
            code: "NOT_FOUND",
            message: "Static asset not found".into(),
            details: json!({}),
        }
        .into_response();
    };
    let content_type = mime_guess::from_path(path).first_or_octet_stream();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type.as_ref())
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(asset.data.into_owned()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tower::ServiceExt;

    async fn response(method: Method, uri: &str, host: &str, body: Option<&str>) -> Response {
        response_with(method, uri, host, body, false, None).await
    }

    async fn response_with(
        method: Method,
        uri: &str,
        host: &str,
        body: Option<&str>,
        authenticated: bool,
        origin: Option<&str>,
    ) -> Response {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, host);
        if body.is_some() {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        if authenticated {
            request = request.header(header::AUTHORIZATION, "Bearer test-token");
        }
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        let state = HttpState::new(test_application().await, "test-token".into(), false);
        router(state)
            .oneshot(
                request
                    .body(Body::from(body.unwrap_or_default().to_owned()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn test_application() -> Arc<BataiApplication> {
        let root = tempfile::tempdir().expect("project").keep();
        std::fs::create_dir_all(root.join(".batai/tasks")).expect("task directory");
        BataiApplication::open(root, crate::application::ApplicationMode::Http)
            .expect("application")
    }

    #[tokio::test]
    async fn health_and_embedded_ui_are_available() {
        assert_eq!(
            response(Method::GET, "/api/health", "127.0.0.1:4317", None)
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            response(Method::GET, "/", "localhost:4317", None)
                .await
                .status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn invalid_host_origin_json_and_missing_token_are_rejected() {
        assert_eq!(
            response(Method::GET, "/api/health", "evil.test", None)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let invalid = response(Method::POST, "/api/tasks", "127.0.0.1:4317", Some("{")).await;
        assert_eq!(
            invalid.status(),
            StatusCode::FORBIDDEN,
            "token is checked before parsing"
        );
        let bytes = to_bytes(invalid.into_body(), 4096).await.expect("body");
        assert!(String::from_utf8_lossy(&bytes).contains("FORBIDDEN"));
        assert_eq!(
            response_with(
                Method::POST,
                "/api/tasks",
                "localhost",
                Some("{"),
                true,
                None
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            response_with(
                Method::GET,
                "/api/health",
                "localhost",
                None,
                false,
                Some("https://evil.test"),
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn oversized_json_is_rejected_before_deserialization() {
        let body = format!("{{\"padding\":\"{}\"}}", "x".repeat(MAX_REQUEST_BYTES + 1));
        let result = response_with(
            Method::POST,
            "/api/tasks",
            "localhost",
            Some(&body),
            true,
            None,
        )
        .await;
        assert_eq!(result.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn golden_http_contract_keeps_all_legacy_routes() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../specs/control-plane-contract-v1.json"))
                .expect("contract fixture");
        assert_eq!(
            fixture["legacyHttpRoutes"].as_array().map(Vec::len),
            Some(9)
        );
        assert!(fixture["legacyHttpRoutes"]
            .to_string()
            .contains("POST /api/tasks/:id/approve"));
    }

    #[tokio::test]
    async fn path_traversal_and_unknown_api_are_not_served() {
        assert_eq!(
            response(Method::GET, "/api/nope", "localhost", None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_ne!(
            response(Method::GET, "/../Cargo.toml", "localhost", None)
                .await
                .status(),
            StatusCode::OK
        );
        let mismatch = response(Method::DELETE, "/api/health", "localhost", None).await;
        assert_eq!(mismatch.status(), StatusCode::METHOD_NOT_ALLOWED);
        let body = to_bytes(mismatch.into_body(), 4096).await.expect("body");
        assert!(String::from_utf8_lossy(&body).contains("METHOD_NOT_ALLOWED"));
    }

    #[tokio::test]
    async fn real_loopback_listener_starts_serves_health_and_stops_gracefully() {
        let application = test_application().await;
        let state = HttpState::new(application, "test-token".into(), false);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            serve_listener(state, listener, async move {
                let _ = shutdown_rx.await;
            })
            .await
        });
        let response = reqwest::get(format!("http://{address}/api/health"))
            .await
            .expect("health request");
        assert_eq!(response.status(), StatusCode::OK);
        shutdown_tx.send(()).expect("shutdown");
        assert!(server.await.expect("server task").is_ok());
    }

    #[tokio::test]
    async fn authenticated_http_mutation_uses_the_shared_rust_task_engine() {
        let root = tempfile::tempdir().expect("project").keep();
        std::fs::create_dir_all(root.join(".batai/agents/director")).expect("agent directory");
        std::fs::create_dir_all(root.join(".batai/tasks")).expect("task directory");
        std::fs::write(
            root.join(".batai/agents/director/config.json"),
            r#"{"id":"director","name":"Director","role_template":"Director","provider":"mock","model":"mock","status":"READY","lifecycle":"PROJECT","authority":"DIRECTOR","parent_agent_id":null}"#,
        )
        .expect("director config");
        let application = BataiApplication::open(root, crate::application::ApplicationMode::Http)
            .expect("application");
        application.start().await.expect("start runtime");
        let state = HttpState::new(application.clone(), "test-token".into(), false);
        let request = Request::builder()
            .method(Method::POST)
            .uri("/api/tasks")
            .header(header::HOST, "127.0.0.1:4317")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .body(Body::from(
                r#"{"id":"http-smoke","objective":"Return a deterministic result","assigned_to":["director"]}"#,
            ))
            .expect("request");
        let response = router(state).oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::CREATED);
        assert!(application
            .runtime()
            .store
            .get_task("http-smoke")
            .expect("task lookup")
            .is_some());
        application.shutdown().await.expect("shutdown");
    }
}
