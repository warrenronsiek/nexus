// @feature agent-traces
// @spec docs/features/agent-traces.md
// @entrypoint integrate
// @boundary authenticated-json

use super::{
    canonical_project_id, origin_permitted, unauthorized, ServerSecret, ServerState,
    NEXUS_PROJECT_HEADER,
};
use crate::agent_traces::{validate_scope, TraceBatch, TraceConflict, MAX_BATCH_BYTES};
use crate::coordination::{DatabaseOperation, NexusService};
use crate::secret_file::OpenedSecretFile;
use anyhow::{ensure, Context, Result};
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{SecondsFormat, TimeDelta, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinHandle;

const APPLICATION_HEADER: &str = "x-nexus-application-id";
const ENVIRONMENT_HEADER: &str = "x-nexus-environment";
const MAX_CREDENTIAL_BYTES: usize = 64 * 1024;
const BODY_SLOTS: usize = 16;
const BODY_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(3);
const PROJECTOR_POLL: Duration = Duration::from_secs(1);
const PROJECTOR_BATCHES: usize = 8;
const EXPIRY_BATCH: usize = 256;
const EXPIRY_INTERVAL: Duration = Duration::from_secs(60 * 60);
const DEFAULT_RETENTION_DAYS: i64 = 30;

#[derive(Clone, Eq, Hash, PartialEq)]
struct Scope {
    project_id: String,
    application_id: String,
    environment: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProducerCredential {
    project_id: String,
    application_id: String,
    environment: String,
    token: String,
}

struct TraceIngress {
    service: Arc<NexusService>,
    credentials: HashMap<Scope, ServerSecret>,
    allowed_origins: Vec<String>,
    body_slots: Arc<Semaphore>,
    write_slots: Arc<Semaphore>,
    wake_projector: Arc<Notify>,
}

pub(super) fn integrate(router: Router, state: &ServerState) -> Result<(Router, TraceProjector)> {
    let service = state.service.clone();
    let ingress = TraceIngress::load(service.clone(), state.allowed_origins.clone())?;
    let projector = TraceProjector::start(service, ingress.as_ref())?;
    let router = match ingress {
        Some(ingress) => router.merge(ingress.router()),
        None => router,
    };
    Ok((router, projector))
}

impl TraceIngress {
    fn load(service: Arc<NexusService>, allowed_origins: Vec<String>) -> Result<Option<Arc<Self>>> {
        let Some(path) =
            std::env::var_os("NEXUS_TRACE_CREDENTIALS_FILE").filter(|path| !path.is_empty())
        else {
            return Ok(None);
        };
        let file = OpenedSecretFile::open(Path::new(&path))
            .context("open NEXUS_TRACE_CREDENTIALS_FILE")?;
        let source = file
            .read_utf8_bounded(MAX_CREDENTIAL_BYTES)
            .context("read NEXUS_TRACE_CREDENTIALS_FILE")?;
        Self::new(service, allowed_origins, &source).map(Some)
    }

    fn new(
        service: Arc<NexusService>,
        allowed_origins: Vec<String>,
        source: &str,
    ) -> Result<Arc<Self>> {
        let entries: Vec<ProducerCredential> = serde_json::from_str(source)
            .map_err(|_| anyhow::anyhow!("invalid trace credential file"))?;
        let mut credentials = HashMap::with_capacity(entries.len());
        for entry in entries {
            let scope = Scope {
                project_id: canonical_project_id(&entry.project_id)
                    .context("invalid trace credential project_id")?,
                application_id: entry.application_id,
                environment: entry.environment,
            };
            validate_scope(&scope.application_id, &scope.environment)?;
            ensure!(
                credentials
                    .insert(scope, ServerSecret::new(entry.token)?)
                    .is_none(),
                "duplicate scope in trace credential file"
            );
        }
        Ok(Arc::new(Self {
            service,
            credentials,
            allowed_origins,
            body_slots: Arc::new(Semaphore::new(BODY_SLOTS)),
            write_slots: Arc::new(Semaphore::new(DatabaseOperation::TraceIngest.capacity())),
            wake_projector: Arc::new(Notify::new()),
        }))
    }

    fn router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/api/v1/agent-traces/events", post(append))
            .with_state(self)
    }

    fn authenticate(&self, headers: &HeaderMap) -> Option<Scope> {
        let scope = Scope::from_headers(headers)?;
        self.credentials
            .get(&scope)?
            .permits(headers)
            .then_some(scope)
    }
}

impl Scope {
    fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let get = |name| headers.get(name).and_then(|header| header.to_str().ok());
        Some(Self {
            project_id: get(NEXUS_PROJECT_HEADER).and_then(canonical_project_id)?,
            application_id: get(APPLICATION_HEADER)?.to_owned(),
            environment: get(ENVIRONMENT_HEADER)?.to_owned(),
        })
    }
}

async fn append(State(ingress): State<Arc<TraceIngress>>, request: Request) -> Response {
    let Some(scope) = ingress.authenticate(request.headers()) else {
        return unauthorized();
    };
    if !origin_permitted(&ingress.allowed_origins, request.headers()) {
        return (StatusCode::FORBIDDEN, "origin is not allowed").into_response();
    }
    record_request(ingress, scope, request.into_body()).await
}

async fn read_batch_body(
    body: axum::body::Body,
) -> Result<axum::body::Bytes, (StatusCode, &'static str)> {
    match tokio::time::timeout(BODY_TIMEOUT, to_bytes(body, MAX_BATCH_BYTES)).await {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(_)) => Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "trace batch exceeds the byte limit",
        )),
        Err(_) => Err((StatusCode::REQUEST_TIMEOUT, "trace batch body timed out")),
    }
}

async fn record_request(
    ingress: Arc<TraceIngress>,
    scope: Scope,
    request_body: axum::body::Body,
) -> Response {
    let Ok(body_slot) = ingress.body_slots.clone().try_acquire_owned() else {
        return overloaded();
    };
    let body = match read_batch_body(request_body).await {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    let Ok(write_slot) = ingress.write_slots.clone().try_acquire_owned() else {
        return overloaded();
    };
    let task = tokio::task::spawn_blocking(move || {
        let _body_slot = body_slot;
        let _write_slot = write_slot;
        append_batch(&ingress, scope, &body)
    });
    match tokio::time::timeout(WRITE_TIMEOUT, task).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) | Err(_) => overloaded(),
    }
}

fn append_batch(ingress: &TraceIngress, scope: Scope, body: &[u8]) -> Response {
    let Ok(batch) = serde_json::from_slice::<TraceBatch>(body) else {
        return (StatusCode::BAD_REQUEST, "invalid trace batch JSON").into_response();
    };
    if batch.application_id != scope.application_id || batch.environment != scope.environment {
        return (
            StatusCode::FORBIDDEN,
            "trace batch does not match the credential scope",
        )
            .into_response();
    }
    if let Err(error) = batch.validate() {
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    match ingress.service.append_trace(&scope.project_id, batch) {
        Ok(receipt) => {
            ingress.wake_projector.notify_one();
            (StatusCode::ACCEPTED, Json(receipt)).into_response()
        }
        Err(error) if error.is::<TraceConflict>() => {
            (StatusCode::CONFLICT, error.to_string()).into_response()
        }
        Err(_) => overloaded(),
    }
}

fn overloaded() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::RETRY_AFTER, "1")],
        "trace ingestion is temporarily unavailable; retry the same batch",
    )
        .into_response()
}

pub(super) struct TraceProjector {
    projection: JoinHandle<()>,
    expiry: JoinHandle<()>,
}

impl TraceProjector {
    fn start(service: Arc<NexusService>, ingress: Option<&Arc<TraceIngress>>) -> Result<Self> {
        let retention = payload_retention()?;
        let expiry = tokio::spawn(expire_payloads(service.clone(), retention));
        let wake = ingress
            .map(|ingress| ingress.wake_projector.clone())
            .unwrap_or_else(|| Arc::new(Notify::new()));
        let projection = tokio::spawn(async move {
            let mut was_unavailable = false;
            loop {
                let service = service.clone();
                let work =
                    tokio::task::spawn_blocking(move || service.project_traces(PROJECTOR_BATCHES))
                        .await;
                let drained = match work {
                    Ok(Ok(count)) => {
                        was_unavailable = false;
                        count < PROJECTOR_BATCHES
                    }
                    Ok(Err(_)) | Err(_) => {
                        if !was_unavailable {
                            eprintln!("Nexus trace projection is temporarily unavailable");
                            was_unavailable = true;
                        }
                        true
                    }
                };
                if drained {
                    tokio::select! {
                        _ = wake.notified() => {},
                        _ = tokio::time::sleep(PROJECTOR_POLL) => {},
                    }
                } else {
                    tokio::task::yield_now().await;
                }
            }
        });
        Ok(Self { projection, expiry })
    }
}

fn payload_retention() -> Result<TimeDelta> {
    let days = match std::env::var("NEXUS_TRACE_PAYLOAD_RETENTION_DAYS") {
        Ok(value) => value
            .parse::<i64>()
            .context("parse NEXUS_TRACE_PAYLOAD_RETENTION_DAYS")?,
        Err(std::env::VarError::NotPresent) => DEFAULT_RETENTION_DAYS,
        Err(error) => return Err(error).context("read NEXUS_TRACE_PAYLOAD_RETENTION_DAYS"),
    };
    ensure!(
        (1..=3650).contains(&days),
        "NEXUS_TRACE_PAYLOAD_RETENTION_DAYS must be between 1 and 3650"
    );
    Ok(TimeDelta::days(days))
}

async fn expire_payloads(service: Arc<NexusService>, retention: TimeDelta) {
    loop {
        let service = service.clone();
        let cutoff = (Utc::now() - retention).to_rfc3339_opts(SecondsFormat::Nanos, true);
        match tokio::task::spawn_blocking(move || {
            service.expire_trace_payloads(&cutoff, EXPIRY_BATCH)
        })
        .await
        {
            Ok(Ok(EXPIRY_BATCH)) => tokio::time::sleep(PROJECTOR_POLL).await,
            Ok(Ok(_)) => tokio::time::sleep(EXPIRY_INTERVAL).await,
            Ok(Err(_)) | Err(_) => {
                eprintln!("Nexus trace payload expiry is temporarily unavailable");
                tokio::time::sleep(EXPIRY_INTERVAL).await;
            }
        }
    }
}

impl Drop for TraceProjector {
    fn drop(&mut self) {
        self.projection.abort();
        self.expiry.abort();
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::persistence::Store;
    use crate::{Config, LoadedConfig};
    use axum::body::Body;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    const TOKEN: &str = "test-trace-producer-token-at-least-32-bytes";

    #[tokio::test]
    async fn scope_and_durable_receipts_are_enforced_at_the_http_boundary() {
        let service = Arc::new(NexusService::from_store(
            LoadedConfig {
                config: Config::default(),
                sources: vec![],
                hash: "trace-test".into(),
            },
            Store::open_memory().unwrap(),
        ));
        let router = TraceIngress::new(service.clone(), vec![], &json!([{
            "project_id": "desk", "application_id": "support", "environment": "production", "token": TOKEN
        }]).to_string()).unwrap().router();
        let batch = json!({
            "schema_version": 1, "batch_id": "batch-1", "application_id": "support", "environment": "production",
            "events": [{
                "event_id": "start-1", "trace_id": "run-1", "agent_id": "assistant",
                "kind": "tool_started", "occurred_at": "2026-11-26T15:00:00Z",
                "invocation_id": "call-1", "name": "lookup_order"
            }]
        });

        let response = post_batch(&router, "other-project", "support", TOKEN, "invalid JSON").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = post_batch(&router, "desk", "support", "wrong", "invalid JSON").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let mut mismatched = batch.clone();
        mismatched["environment"] = json!("staging");
        let response = post_batch(&router, "desk", "support", TOKEN, &mismatched.to_string()).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = post_batch(&router, "desk", "support", TOKEN, &batch.to_string()).await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let first = response_body(response).await;
        assert_eq!(first["accepted_events"], 1);
        let response = post_batch(&router, "desk", "support", TOKEN, &batch.to_string()).await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert_eq!(response_body(response).await, first);
        let mut changed = batch;
        changed["events"][0]["name"] = json!("different");
        let response = post_batch(&router, "desk", "support", TOKEN, &changed.to_string()).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(service.project_traces(8).unwrap(), 1);
        assert_eq!(service.project_traces(8).unwrap(), 0);
    }

    async fn post_batch(
        router: &Router,
        project: &str,
        app: &str,
        token: &str,
        body: &str,
    ) -> Response {
        let request = Request::post("/api/v1/agent-traces/events")
            .header(NEXUS_PROJECT_HEADER, project)
            .header(APPLICATION_HEADER, app)
            .header(ENVIRONMENT_HEADER, "production")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body.to_owned()))
            .unwrap();
        router.clone().oneshot(request).await.unwrap()
    }

    async fn response_body(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
}
