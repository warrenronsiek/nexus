// @feature usage-analytics
// @spec docs/features/usage-analytics.md
// @feature runtime
// @feature configuration
// @feature sos
// @spec docs/features/runtime.md
// @spec docs/features/configuration.md
// @spec docs/features/sos.md
// @entrypoint NexusClient
// @boundary authenticated-remote-json
use super::{daemon, protocol::WireRequest};
use crate::config::{ClientConfig, LoadedConfig, RemoteClientConfig};
use crate::coordination::api::{PapercutCommand, ServiceRequest, SosCommand};
use crate::coordination::client_evidence::{ClientEvidencePlan, ClientFilesystemEvidence};
use crate::coordination::domain::HookResponse;
use crate::secret_file::{validate_bearer_secret, OpenedSecretFile};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const LIFECYCLE_RESPONSE_BUDGET: Duration = Duration::from_millis(2_500);
const REMOTE_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MODEL_REQUEST_MARGIN: Duration = Duration::from_secs(5);
const CLIENT_EVIDENCE_CAPTURE_BUDGET: Duration = Duration::from_millis(250);
const SECRET_FILE_READ_BUDGET: Duration = Duration::from_secs(2);
const MAX_REMOTE_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemoteResponsePolicy {
    Bounded,
    CompleteRegistry,
}

impl RemoteResponsePolicy {
    fn for_request(request: &ServiceRequest) -> Self {
        if matches!(
            request,
            ServiceRequest::Papercuts(PapercutCommand::Dump { .. })
                | ServiceRequest::Sos(SosCommand::Dump { .. })
        ) {
            Self::CompleteRegistry
        } else {
            Self::Bounded
        }
    }

    async fn decode(self, response: reqwest::Response) -> Result<Value> {
        match self {
            Self::Bounded => decode_bounded_remote_response(response).await,
            Self::CompleteRegistry => response
                .json()
                .await
                .context("decode complete remote issue registry"),
        }
    }
}

enum LocalServicePolicy {
    RequireRunning,
    EnsureRunning,
}

pub struct NexusClient {
    analytics: Option<super::analytics::ClientAnalytics>,
    loaded: Arc<LoadedConfig>,
    explicit_config: Option<PathBuf>,
    http: reqwest::Client,
    request_timeout: Duration,
    secret: tokio::sync::OnceCell<std::result::Result<Secret, String>>,
    evidence_capture: BlockingCaptureGate,
    secret_capture: BlockingCaptureGate,
}

impl NexusClient {
    pub fn new(loaded: &LoadedConfig, explicit_config: Option<&Path>) -> Self {
        Self::from_shared(
            Arc::new(loaded.clone()),
            explicit_config.map(Path::to_path_buf),
        )
    }

    pub(crate) fn from_shared(loaded: Arc<LoadedConfig>, explicit_config: Option<PathBuf>) -> Self {
        let mut client = Self::without_analytics(loaded.clone(), explicit_config.clone());
        client.analytics = super::analytics::ClientAnalytics::start(loaded, explicit_config);
        client
    }

    pub(crate) fn without_analytics(
        loaded: Arc<LoadedConfig>,
        explicit_config: Option<PathBuf>,
    ) -> Self {
        Self {
            analytics: None,
            loaded,
            explicit_config,
            http: reqwest::Client::builder()
                .connect_timeout(REMOTE_CONNECT_TIMEOUT)
                .build()
                .expect("static Nexus HTTP client configuration is valid"),
            request_timeout: REMOTE_REQUEST_TIMEOUT,
            secret: tokio::sync::OnceCell::new(),
            evidence_capture: BlockingCaptureGate::default(),
            secret_capture: BlockingCaptureGate::default(),
        }
    }

    pub async fn request(&self, request: &ServiceRequest) -> Result<Value> {
        self.send_request(request, LocalServicePolicy::RequireRunning)
            .await
    }
    pub async fn ensure_and_request(&self, request: &ServiceRequest) -> Result<Value> {
        self.send_request(request, LocalServicePolicy::EnsureRunning)
            .await
    }
    async fn send_request(
        &self,
        request: &ServiceRequest,
        policy: LocalServicePolicy,
    ) -> Result<Value> {
        let result = match &self.loaded.config.client {
            ClientConfig::Local => match policy {
                LocalServicePolicy::RequireRunning => {
                    daemon::request(&self.loaded.config.runtime.socket_path, request).await
                }
                LocalServicePolicy::EnsureRunning => {
                    daemon::ensure_and_request(
                        &self.loaded,
                        self.explicit_config.as_deref(),
                        request,
                    )
                    .await
                }
            },
            ClientConfig::Remote(remote) => self.remote_request(remote, request).await,
        };
        if result.as_ref().is_ok_and(|value| {
            value.get("recorded").and_then(Value::as_bool) == Some(true)
                || value.get("ok").and_then(Value::as_bool) == Some(true)
        }) {
            if let Some(analytics) = &self.analytics {
                analytics.observe(request);
            }
        }
        result
    }

    pub async fn lifecycle_request(&self, request: &ServiceRequest) -> Value {
        let outcome =
            tokio::time::timeout(LIFECYCLE_RESPONSE_BUDGET, self.ensure_and_request(request)).await;
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

    pub(crate) fn is_remote(&self) -> bool {
        matches!(self.loaded.config.client, ClientConfig::Remote(_))
    }

    async fn remote_request(
        &self,
        remote: &RemoteClientConfig,
        request: &ServiceRequest,
    ) -> Result<Value> {
        let timeout = self.remote_request_timeout(request);
        let response_policy = RemoteResponsePolicy::for_request(request);
        let mut wire = WireRequest::from_service(request, Some(remote.project_id.trim()))?;
        if let Some(plan) = ClientEvidencePlan::for_request(request) {
            wire.client_evidence = self
                .evidence_capture
                .capture(CLIENT_EVIDENCE_CAPTURE_BUDGET, move || plan.capture())
                .await
                .flatten();
        }
        self.send_remote(remote, &wire, timeout, response_policy)
            .await
    }

    pub(crate) async fn submit_reconciliation(
        &self,
        evidence: ClientFilesystemEvidence,
    ) -> Result<()> {
        let ClientConfig::Remote(remote) = &self.loaded.config.client else {
            return Ok(());
        };
        evidence.validate_reconciliation()?;
        self.send_remote(
            remote,
            &WireRequest::reconciliation(remote.project_id.trim(), evidence),
            REMOTE_REQUEST_TIMEOUT,
            RemoteResponsePolicy::Bounded,
        )
        .await?;
        Ok(())
    }

    async fn send_remote(
        &self,
        remote: &RemoteClientConfig,
        request: &WireRequest,
        timeout: Duration,
        response_policy: RemoteResponsePolicy,
    ) -> Result<Value> {
        let secret = self.remote_secret(remote).await?;
        let endpoint = format!(
            "{}/api/v1/request",
            remote.endpoint.trim().trim_end_matches('/')
        );
        let response = self
            .http
            .post(endpoint)
            .bearer_auth(secret.expose())
            .json(request)
            .timeout(timeout)
            .send()
            .await
            .context("connect to remote Nexus service")?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            bail!("remote Nexus rejected the configured bearer secret");
        }
        if !status.is_success() {
            bail!("remote Nexus request failed with HTTP {status}");
        }
        response_policy.decode(response).await
    }

    fn remote_request_timeout(&self, request: &ServiceRequest) -> Duration {
        match request {
            ServiceRequest::Analyze(_) => Duration::from_secs(
                self.loaded
                    .config
                    .analyst
                    .timeout_seconds
                    .saturating_add(MODEL_REQUEST_MARGIN.as_secs()),
            ),
            _ => self.request_timeout,
        }
    }

    async fn remote_secret<'a>(&'a self, remote: &'a RemoteClientConfig) -> Result<&'a Secret> {
        let loaded = self
            .secret
            .get_or_init(|| async {
                load_remote_secret(remote, &self.secret_capture)
                    .await
                    .map_err(|error| format!("{error:#}"))
            })
            .await;
        loaded
            .as_ref()
            .map_err(|error| anyhow::anyhow!(error.clone()))
    }
}

async fn decode_bounded_remote_response(mut response: reqwest::Response) -> Result<Value> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_REMOTE_RESPONSE_BYTES as u64)
    {
        bail!("remote Nexus response exceeds {MAX_REMOTE_RESPONSE_BYTES} bytes");
    }
    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(MAX_REMOTE_RESPONSE_BYTES as u64) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .context("read remote Nexus response")?
    {
        let remaining_with_sentinel = (MAX_REMOTE_RESPONSE_BYTES + 1).saturating_sub(bytes.len());
        let copied = chunk.len().min(remaining_with_sentinel);
        bytes.extend_from_slice(&chunk[..copied]);
        if bytes.len() > MAX_REMOTE_RESPONSE_BYTES || copied < chunk.len() {
            bail!("remote Nexus response exceeds {MAX_REMOTE_RESPONSE_BYTES} bytes");
        }
    }
    serde_json::from_slice(&bytes).context("decode remote Nexus response")
}

#[derive(Clone)]
struct BlockingCaptureGate {
    slot: Arc<tokio::sync::Semaphore>,
}

impl Default for BlockingCaptureGate {
    fn default() -> Self {
        Self {
            slot: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
}

impl BlockingCaptureGate {
    async fn capture<T, F>(&self, budget: Duration, capture: F) -> Option<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let permit = self.slot.clone().try_acquire_owned().ok()?;
        let worker = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            capture()
        });
        tokio::time::timeout(budget, worker).await.ok()?.ok()
    }
}

struct Secret(String);

impl Secret {
    fn expose(&self) -> &str {
        &self.0
    }
}

async fn load_remote_secret(
    remote: &RemoteClientConfig,
    capture_gate: &BlockingCaptureGate,
) -> Result<Secret> {
    let value = if let Some(value) = std::env::var_os("NEXUS_SECRET") {
        value.to_string_lossy().into_owned()
    } else if let Some(path) = &remote.secret_file {
        let path = path.clone();
        let display = path.display().to_string();
        capture_gate
            .capture(SECRET_FILE_READ_BUDGET, move || {
                OpenedSecretFile::open(&path).and_then(OpenedSecretFile::read_utf8)
            })
            .await
            .context("Nexus secret file read timed out or another read is still active")?
            .with_context(|| format!("read Nexus secret file {display}"))?
    } else {
        bail!("remote Nexus requires NEXUS_SECRET or client.secret_file");
    };
    let value = value.trim().to_owned();
    validate_bearer_secret(&value)?;
    Ok(Secret(value))
}

#[cfg(test)]
mod tests;
