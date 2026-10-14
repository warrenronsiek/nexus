// @feature runtime
// @feature analyst
// @feature sos
// @spec docs/features/runtime.md
// @spec docs/features/analyst.md
// @spec docs/features/sos.md
// @entrypoint serve_listener
// @boundary dynamic-json
mod listener;
mod mcp_http;
mod secret;

pub use secret::{load_or_generate_secret, LoadedServerSecret, ServerSecret};

use super::{dispatch, protocol::WireRequest};
use crate::agents::model;
use crate::coordination::api::{ServiceRequest, ServiceResponse};
use crate::coordination::client_evidence::ClientFilesystemEvidence;
use crate::coordination::domain::HookResponse;
use crate::coordination::NexusService;
use anyhow::{Context, Result};
use axum::body::{to_bytes, Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

const MAX_REMOTE_REQUEST_BYTES: usize = 1024 * 1024;
const REMOTE_BODY_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_QUEUE_TIMEOUT: Duration = Duration::from_secs(1);
const READINESS_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_IN_FLIGHT_REMOTE_REQUESTS: usize = 64;
pub(super) const NEXUS_PROJECT_HEADER: &str = "x-nexus-project-id";
pub(super) const NEXUS_AGENT_HEADER: &str = "x-nexus-agent";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SosHttpReport {
    message: String,
    #[serde(rename = "type")]
    kind: String,
    environment: String,
    version: String,
    model: String,
    trace: String,
    metadata: Map<String, Value>,
}

pub fn disable_in_process_model_providers() {
    model::disable_in_process_provider_execution();
}

struct ServerState {
    service: Arc<NexusService>,
    model_work: dispatch::ModelWorkScheduler,
    secret: ServerSecret,
    allowed_origins: Vec<String>,
    mcp_sessions: mcp_http::McpSessionRegistry,
    remote_request_slots: Arc<Semaphore>,
    blocking_execution: dispatch::BlockingExecutionGate,
    readiness_slot: Arc<Semaphore>,
}

#[derive(Clone)]
struct RemoteDispatcher {
    service: Arc<NexusService>,
    model_work: dispatch::ModelWorkScheduler,
    blocking_execution: dispatch::BlockingExecutionGate,
    project_id: String,
}

impl RemoteDispatcher {
    fn new(state: &ServerState, project_id: String) -> Self {
        Self {
            service: state.service.clone(),
            model_work: state.model_work.clone(),
            blocking_execution: state.blocking_execution.clone(),
            project_id,
        }
    }

    async fn dispatch(
        self,
        request: ServiceRequest,
        evidence: Option<ClientFilesystemEvidence>,
        lifecycle: bool,
    ) -> ServiceResponse {
        let dispatch = async move {
            match evidence {
                Some(evidence) => {
                    dispatch::handle_observed_bounded(
                        self.service,
                        &self.model_work,
                        &self.blocking_execution,
                        request,
                        self.project_id,
                        evidence,
                    )
                    .await
                }
                None => {
                    dispatch::handle_bounded(
                        self.service,
                        &self.model_work,
                        &self.blocking_execution,
                        request,
                        Some(self.project_id),
                    )
                    .await
                }
            }
        };
        if !lifecycle {
            return dispatch.await;
        }
        tokio::time::timeout(super::client::LIFECYCLE_RESPONSE_BUDGET, dispatch)
            .await
            .unwrap_or_else(|_| {
                ServiceResponse::Hook(HookResponse::fail_open(format!(
                    "nexus did not respond within {}ms",
                    super::client::LIFECYCLE_RESPONSE_BUDGET.as_millis()
                )))
            })
    }

    async fn dispatch_wire(
        self,
        method: String,
        params: Value,
        evidence: Option<ClientFilesystemEvidence>,
    ) -> std::result::Result<ServiceResponse, Box<Response>> {
        let lifecycle = ServiceRequest::is_lifecycle_method(&method);
        let request = match ServiceRequest::decode(&method, params) {
            Ok(request) => request,
            Err(error) if lifecycle => {
                return Ok(ServiceResponse::Hook(HookResponse::fail_open(error)))
            }
            Err(error) => return Ok(ServiceResponse::error(error)),
        };
        if evidence
            .as_ref()
            .is_some_and(|evidence| evidence.validate_for(&request).is_err())
        {
            return Err(Box::new(
                (StatusCode::BAD_REQUEST, "invalid client evidence").into_response(),
            ));
        }
        Ok(self.dispatch(request, evidence, lifecycle).await)
    }
}

pub async fn serve_listener(
    listener: tokio::net::TcpListener,
    service: Arc<NexusService>,
    secret: ServerSecret,
    allowed_origins: Vec<String>,
) -> Result<()> {
    let _memory_scheduler = dispatch::MemoryConsolidationScheduler::start(service.clone());
    let model_work = dispatch::ModelWorkScheduler::start(service.clone());
    let state = Arc::new(ServerState {
        service,
        model_work,
        secret,
        allowed_origins,
        mcp_sessions: mcp_http::McpSessionRegistry::default(),
        remote_request_slots: Arc::new(Semaphore::new(MAX_IN_FLIGHT_REMOTE_REQUESTS)),
        blocking_execution: dispatch::BlockingExecutionGate::new(
            MAX_IN_FLIGHT_REMOTE_REQUESTS,
            REMOTE_QUEUE_TIMEOUT,
        ),
        readiness_slot: Arc::new(Semaphore::new(1)),
    });
    let router = Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(ready))
        .route("/api/v1/request", post(api_request))
        .route("/api/v1/sos", post(api_sos))
        .route(
            "/mcp",
            post(mcp_http::request)
                .get(mcp_http::get)
                .delete(mcp_http::delete),
        )
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state,
            protect_remote_request,
        ));
    listener::serve(listener, router)
        .await
        .context("serve Nexus HTTP endpoint")
}

async fn protect_remote_request(
    State(state): State<Arc<ServerState>>,
    request: Request,
    next: Next,
) -> Response {
    if !matches!(
        request.uri().path(),
        "/api/v1/request" | "/api/v1/sos" | "/mcp"
    ) {
        return next.run(request).await;
    }
    if let Some(response) = authenticated_request_rejection(&state, request.headers()) {
        return response;
    }
    let permit = match tokio::time::timeout(
        REMOTE_QUEUE_TIMEOUT,
        state.remote_request_slots.clone().acquire_owned(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        _ => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Nexus request capacity is temporarily exhausted",
            )
                .into_response()
        }
    };
    let (parts, body) = request.into_parts();
    let body = match tokio::time::timeout(
        REMOTE_BODY_TIMEOUT,
        to_bytes(body, MAX_REMOTE_REQUEST_BYTES),
    )
    .await
    {
        Ok(Ok(body)) => body,
        Ok(Err(_)) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "request body is too large").into_response()
        }
        Err(_) => return (StatusCode::REQUEST_TIMEOUT, "request body timed out").into_response(),
    };
    let response = next.run(Request::from_parts(parts, Body::from(body))).await;
    drop(permit);
    response
}

async fn health() -> Json<Value> {
    Json(json!({"status":"running"}))
}

async fn ready(State(state): State<Arc<ServerState>>) -> Response {
    let service = state.service.clone();
    bounded_readiness_response(state.readiness_slot.clone(), READINESS_TIMEOUT, move || {
        service.readiness_status()
    })
    .await
}

async fn bounded_readiness_response<F>(
    readiness_slot: Arc<Semaphore>,
    timeout: Duration,
    check: F,
) -> Response
where
    F: FnOnce() -> Result<&'static str> + Send + 'static,
{
    let Ok(permit) = readiness_slot.try_acquire_owned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status":"busy"})),
        )
            .into_response();
    };
    let probe = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        check()
    });
    match tokio::time::timeout(timeout, probe).await {
        Ok(Ok(Ok(status))) => (StatusCode::OK, Json(json!({"status":status}))).into_response(),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status":"unavailable"})),
        )
            .into_response(),
    }
}

async fn api_request(State(state): State<Arc<ServerState>>, body: Bytes) -> Response {
    let wire = match serde_json::from_slice::<WireRequest>(&body) {
        Ok(request) => request,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid request").into_response(),
    };
    let WireRequest {
        method,
        params,
        project_id,
        client_evidence,
    } = wire;
    let Some(project_id) = project_id.as_deref().and_then(canonical_project_id) else {
        return (StatusCode::BAD_REQUEST, "project_id is required").into_response();
    };
    if method == "client_reconcile" {
        return reconcile_client_evidence(
            state.service.clone(),
            state.blocking_execution.clone(),
            project_id,
            client_evidence,
        )
        .await;
    }
    match RemoteDispatcher::new(&state, project_id)
        .dispatch_wire(method, params, client_evidence)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response.to_json())).into_response(),
        Err(response) => *response,
    }
}

async fn api_sos(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(project_id) = headers
        .get(NEXUS_PROJECT_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(canonical_project_id)
    else {
        return (
            StatusCode::BAD_REQUEST,
            "X-Nexus-Project-ID header is required",
        )
            .into_response();
    };
    let report = match serde_json::from_slice::<SosHttpReport>(&body) {
        Ok(report) => report,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid SOS report").into_response(),
    };
    let agent = match optional_bounded_header(
        &headers,
        NEXUS_AGENT_HEADER,
        crate::coordination::api::MAX_AGENT_BYTES,
    ) {
        Ok(agent) => agent.unwrap_or_else(|| "sos-http".to_owned()),
        Err(()) => {
            return (StatusCode::BAD_REQUEST, "invalid X-Nexus-Agent header").into_response()
        }
    };
    let params = json!({
        "action":"report",
        "session_id":format!("sos-http-{}", uuid::Uuid::new_v4()),
        "agent":agent,
        "message":report.message,
        "type":report.kind,
        "environment":report.environment,
        "version":report.version,
        "model":report.model,
        "trace":report.trace,
        "metadata":report.metadata
    });
    let request = match ServiceRequest::decode("sos", params) {
        Ok(request) => request,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid SOS report").into_response(),
    };
    let response = RemoteDispatcher::new(&state, project_id)
        .dispatch(request, None, false)
        .await;
    match response {
        response @ ServiceResponse::Sos(_) => {
            (StatusCode::ACCEPTED, Json(response.to_json())).into_response()
        }
        response => (StatusCode::BAD_REQUEST, Json(response.to_json())).into_response(),
    }
}

async fn reconcile_client_evidence(
    service: Arc<NexusService>,
    blocking_execution: dispatch::BlockingExecutionGate,
    project_id: String,
    evidence: Option<ClientFilesystemEvidence>,
) -> Response {
    let Some(evidence) = evidence.filter(|evidence| evidence.validate_reconciliation().is_ok())
    else {
        return (
            StatusCode::BAD_REQUEST,
            "invalid or missing client evidence",
        )
            .into_response();
    };
    let truncated = evidence.changed_paths_truncated;
    let result = blocking_execution
        .run(move || service.reconcile_remote_evidence(&project_id, &evidence, true))
        .await;
    match result {
        Ok(Ok(observed)) => (
            StatusCode::OK,
            Json(json!({
                "ok":true,
                "observed":observed,
                "truncated":truncated
            })),
        )
            .into_response(),
        Ok(Err(error)) => service_unavailable(error),
        Err(error) => service_unavailable(error),
    }
}

fn service_unavailable(error: impl std::fmt::Display) -> Response {
    eprintln!("Nexus remote request failed internally: {error}");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"ok":false,"error":"Nexus service is temporarily unavailable"})),
    )
        .into_response()
}

fn canonical_project_id(value: &str) -> Option<String> {
    crate::config::canonical_project_id(value)
        .ok()
        .map(str::to_owned)
}

fn optional_bounded_header(
    headers: &HeaderMap,
    name: &str,
    maximum: usize,
) -> std::result::Result<Option<String>, ()> {
    let Some(value) = headers.get(name) else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| ())?.trim();
    if value.is_empty() || value.len() > maximum {
        return Err(());
    }
    Ok(Some(value.to_owned()))
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        "authentication required",
    )
        .into_response()
}

fn authenticated_request_rejection(state: &ServerState, headers: &HeaderMap) -> Option<Response> {
    if !state.secret.permits(headers) {
        Some(unauthorized())
    } else if !origin_permitted(&state.allowed_origins, headers) {
        Some((StatusCode::FORBIDDEN, "origin is not allowed").into_response())
    } else {
        None
    }
}

fn origin_permitted(allowed_origins: &[String], headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    allowed_origins.iter().any(|allowed| allowed == origin)
}

#[cfg(test)]
mod tests {
    use super::{bounded_readiness_response, canonical_project_id, origin_permitted};
    use axum::http::{header, HeaderMap, HeaderValue};
    use std::sync::{mpsc, Arc};

    #[test]
    fn project_ids_reject_control_characters_at_the_http_boundary() {
        assert_eq!(
            canonical_project_id(" shared-project ").as_deref(),
            Some("shared-project")
        );
        assert_eq!(canonical_project_id("shared\0project"), None);
        assert_eq!(canonical_project_id("shared\nproject"), None);
    }
    use std::time::{Duration, Instant};
    use tokio::sync::Semaphore;

    #[test]
    fn malformed_origin_cannot_bypass_the_allowlist() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, HeaderValue::from_bytes(b"\xff").unwrap());

        assert!(!origin_permitted(
            &["https://agent.example.test".into()],
            &headers
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn timed_out_readiness_probe_keeps_single_flight_ownership_until_it_returns() {
        let slot = Arc::new(Semaphore::new(1));
        let (started_sender, started_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let first_slot = slot.clone();
        let started_at = Instant::now();
        let first = tokio::spawn(async move {
            bounded_readiness_response(first_slot, Duration::from_millis(25), move || {
                started_sender.send(()).unwrap();
                release_receiver.recv().unwrap();
                Ok("ready")
            })
            .await
        });
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("the readiness check must start");

        let response = first.await.unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(started_at.elapsed() < Duration::from_millis(500));
        assert_eq!(slot.available_permits(), 0);

        let second_started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let second_started_in_probe = second_started.clone();
        let second =
            bounded_readiness_response(slot.clone(), Duration::from_millis(25), move || {
                second_started_in_probe.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok("ready")
            })
            .await;
        assert_eq!(second.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert!(!second_started.load(std::sync::atomic::Ordering::SeqCst));

        release_sender.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while slot.available_permits() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the probe slot must be released when the blocking check returns");
    }
}
