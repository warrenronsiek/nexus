// @feature observability-ui
// @feature runtime
// @feature usage-analytics
// @spec docs/features/observability-ui.md
// @spec docs/features/runtime.md
// @spec docs/features/usage-analytics.md
// @entrypoint spawn
// @boundary dynamic-http
use crate::config::{LoadedConfig, UiConfig};
use crate::coordination::api::{DashboardQuery, ServiceRequest, ServiceResponse, UsageQuery};
use crate::coordination::NexusService;
use crate::runtime::dispatch;
use anyhow::{bail, Context, Result};
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

const INDEX: &str = include_str!("../../ui/dist/index.html");
const ELM: &str = include_str!("../../ui/dist/elm.js");
const APP: &str = include_str!("../../ui/dist/app.js");
const STYLES: &str = include_str!("../../ui/dist/styles.css");

#[derive(Clone)]
struct WebState {
    service: Arc<NexusService>,
}

#[derive(Deserialize, Serialize)]
struct HealthResponse {
    status: String,
    mode: String,
}

pub async fn spawn(config: &UiConfig, service: Arc<NexusService>) -> Option<JoinHandle<()>> {
    if !config.enabled {
        return None;
    }
    let listener = match TcpListener::bind(config.bind_address).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!(
                "warning: Nexus UI unavailable at {}: {error}",
                config.bind_address
            );
            return None;
        }
    };
    let address = config.bind_address;
    Some(tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router(service, address)).await {
            eprintln!("warning: Nexus UI stopped: {error}");
        }
    }))
}

fn url(config: &UiConfig) -> String {
    format!("http://{}", config.bind_address)
}

pub async fn ensure_dashboard(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
) -> Result<String> {
    if !loaded.config.ui.enabled {
        bail!("Nexus UI is disabled by configuration");
    }
    super::daemon::ensure_and_request(loaded, explicit_config, &ServiceRequest::Status).await?;
    wait_until_ready(loaded.config.ui.bind_address, Duration::from_secs(2)).await?;
    Ok(url(&loaded.config.ui))
}

async fn wait_until_ready(address: SocketAddr, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let attempt = remaining.min(Duration::from_millis(200));
        if matches!(
            tokio::time::timeout(attempt, health_check(address)).await,
            Ok(Ok(()))
        ) {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        tokio::time::sleep(remaining.min(Duration::from_millis(50))).await;
    }
    bail!("Nexus UI did not become ready at http://{address}")
}

fn router(service: Arc<NexusService>, address: SocketAddr) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/assets/elm.js", get(elm))
        .route("/assets/app.js", get(app))
        .route("/assets/styles.css", get(styles))
        .route("/api/v1/health", get(health))
        .route("/api/v1/projects", get(projects))
        .route("/api/v1/dashboard", get(dashboard))
        .route("/api/v1/usage", get(usage))
        .layer(middleware::from_fn_with_state(
            address,
            secure_local_request,
        ))
        .with_state(WebState { service })
}

async fn secure_local_request(
    State(address): State<SocketAddr>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let accepted = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| accepted_host(host, address));
    let mut response = if accepted {
        next.run(request).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    };
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn accepted_host(host: &str, address: SocketAddr) -> bool {
    host == address.to_string()
        || host == address.ip().to_string()
        || matches!(address.ip(), IpAddr::V6(ip) if host == format!("[{ip}]"))
        || host == format!("localhost:{}", address.port())
        || host == "localhost"
}

async fn index() -> Html<&'static str> {
    Html(INDEX)
}

async fn elm() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        ELM,
    )
}

async fn app() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        APP,
    )
}

async fn styles() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], STYLES)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "running".into(),
        mode: "read_only".into(),
    })
}

async fn projects(State(state): State<WebState>) -> Response {
    handle_api(state, ServiceRequest::Projects).await
}

async fn dashboard(State(state): State<WebState>, Query(query): Query<DashboardQuery>) -> Response {
    handle_api(state, ServiceRequest::Dashboard(query)).await
}

async fn usage(State(state): State<WebState>, Query(query): Query<UsageQuery>) -> Response {
    handle_api(state, ServiceRequest::Usage(query)).await
}

async fn handle_api(state: WebState, request: ServiceRequest) -> Response {
    api_response(dispatch::handle(state.service, request).await)
}

fn api_response(response: ServiceResponse) -> Response {
    let status = if matches!(response, ServiceResponse::Error(_)) {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    };
    (status, Json(response.to_json())).into_response()
}

async fn health_check(address: SocketAddr) -> Result<()> {
    let mut stream = TcpStream::connect(address).await?;
    stream
        .write_all(
            format!("GET /api/v1/health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let status = response
        .split(|byte| *byte == b'\n')
        .next()
        .context("UI health response was empty")?;
    if !status.starts_with(b"HTTP/1.1 200") {
        bail!("UI health endpoint was not ready")
    }
    let body_start = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
        .context("UI health response had no body")?;
    let probe: HealthResponse = serde_json::from_slice(&response[body_start..])?;
    if probe.status == "running" && probe.mode == "read_only" {
        Ok(())
    } else {
        bail!("UI health endpoint returned an unexpected service")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_an_ipv6_loopback_host_without_a_default_port() {
        let address = SocketAddr::new("::1".parse().unwrap(), 80);

        assert!(accepted_host("[::1]", address));
    }

    #[tokio::test]
    async fn readiness_rejects_an_unrelated_http_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut request = [0; 1024];
                    let _ = stream.read(&mut request).await;
                    let body = br#"{"status":"something_else"}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                    stream.write_all(body).await.unwrap();
                });
            }
        });

        let result = wait_until_ready(address, Duration::from_millis(150)).await;
        server.abort();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn readiness_is_bounded_when_a_listener_never_responds() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });

        let result = tokio::time::timeout(
            Duration::from_secs(1),
            wait_until_ready(address, Duration::from_millis(150)),
        )
        .await;
        server.abort();
        assert!(result.is_ok(), "readiness exceeded its outer bound");
        assert!(result.unwrap().is_err());
    }
}
