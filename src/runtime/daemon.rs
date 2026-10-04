// @feature runtime
// @spec docs/features/runtime.md
// @entrypoint serve
// @boundary dynamic-json
use crate::config::LoadedConfig;
use crate::coordination::api::{ServiceRequest, ServiceResponse};
use crate::coordination::domain::HookResponse;
use crate::coordination::NexusService;
use crate::runtime::web;
use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    method: String,
    #[serde(default)]
    params: Value,
}

pub async fn serve(loaded: LoadedConfig) -> Result<()> {
    let socket_path = loaded.config.runtime.socket_path.clone();
    let lock_path = loaded.config.runtime.lock_path.clone();
    let reconcile_seconds = loaded.config.coordination.reconcile_seconds;
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if let Some(parent) = lock_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    lock.try_lock_exclusive()
        .with_context(|| format!("another Nexus daemon owns {}", lock_path.display()))?;

    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("bind {}", socket_path.display()))?;
    let ui_config = loaded.config.ui.clone();
    let service = Arc::new(NexusService::new(loaded)?);
    let ui = web::spawn(&ui_config, service.clone()).await;
    let mut reconcile = reconciliation_interval(reconcile_seconds);

    loop {
        tokio::select! {
            incoming = listener.accept() => {
                let (stream, _) = incoming?;
                let service = service.clone();
                tokio::spawn(async move { let _ = handle_connection(stream, service).await; });
            }
            _ = reconcile.tick() => {
                let service = service.clone();
                let _ = tokio::task::spawn_blocking(move || service.reconcile_all()).await;
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    let _ = std::fs::remove_file(&socket_path);
    if let Some(ui) = ui {
        ui.abort();
    }
    drop(lock);
    Ok(())
}

fn reconciliation_interval(seconds: u64) -> tokio::time::Interval {
    let period = Duration::from_secs(seconds);
    let start = tokio::time::Instant::now() + period;
    let mut interval = tokio::time::interval_at(start, period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}

async fn handle_connection(stream: UnixStream, service: Arc<NexusService>) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => match ServiceRequest::decode(&request.method, request.params) {
                Ok(request) => {
                    let service = service.clone();
                    tokio::task::spawn_blocking(move || service.handle(request))
                        .await
                        .context("join service request worker")?
                }
                Err(error) if ServiceRequest::is_lifecycle_method(&request.method) => {
                    ServiceResponse::Hook(HookResponse::fail_open(error))
                }
                Err(error) => ServiceResponse::error(error),
            },
            Err(error) => ServiceResponse::error(format!("invalid request: {error}")),
        };
        writer
            .write_all(serde_json::to_string(&response)?.as_bytes())
            .await?;
        writer.write_all(b"\n").await?;
    }
    Ok(())
}

pub async fn request(socket_path: &Path, request: &ServiceRequest) -> Result<Value> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connect {}", socket_path.display()))?;
    let (method, params) = request.wire_parts()?;
    let request = Request {
        method: method.to_owned(),
        params,
    };
    stream
        .write_all(serde_json::to_string(&request)?.as_bytes())
        .await?;
    stream.write_all(b"\n").await?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).await?;
    serde_json::from_str(&line).context("decode daemon response")
}

/// Lifecycle hooks must answer before every generated host hook timeout, including daemon
/// start-up, so a slow or wedged daemon degrades to an unrecorded observation.
pub(crate) const LIFECYCLE_RESPONSE_BUDGET: Duration = Duration::from_millis(2_500);

pub async fn lifecycle_request(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    request: &ServiceRequest,
) -> Value {
    let outcome = tokio::time::timeout(
        LIFECYCLE_RESPONSE_BUDGET,
        ensure_and_request(loaded, explicit_config, request),
    )
    .await;
    let diagnostic = match outcome {
        Ok(Ok(response)) => return response,
        Ok(Err(error)) => format!("nexus is unavailable: {error}"),
        Err(_) => format!(
            "nexus did not respond within {}ms",
            LIFECYCLE_RESPONSE_BUDGET.as_millis()
        ),
    };
    serde_json::to_value(HookResponse::fail_open(diagnostic)).expect("hook responses serialize")
}

pub async fn ensure_and_request(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    request_value: &ServiceRequest,
) -> Result<Value> {
    if let Ok(response) = request(&loaded.config.runtime.socket_path, request_value).await {
        return Ok(response);
    }

    let executable = std::env::current_exe()?;
    let mut command = tokio::process::Command::new(executable);
    if let Some(path) = explicit_config {
        command.arg("--config").arg(path);
    }
    command
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.spawn().context("start Nexus daemon")?;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Ok(response) = request(&loaded.config.runtime.socket_path, request_value).await {
            return Ok(response);
        }
    }
    anyhow::bail!(
        "Nexus daemon did not become ready at {}",
        loaded.config.runtime.socket_path.display()
    )
}
