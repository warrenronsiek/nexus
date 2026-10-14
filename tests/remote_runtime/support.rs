// @feature runtime
// @feature configuration
// @feature coordination
// @feature usage-analytics
// @spec docs/features/runtime.md
// @spec docs/features/configuration.md
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
// @boundary dynamic-json
#[path = "../support/stdio_mcp.rs"]
mod stdio_mcp;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use axum::{Json, Router};
use nexus::config::{ClientConfig, Config, LoadedConfig, RemoteClientConfig};
use nexus::coordination::api::ServiceRequest;
use nexus::runtime::client::NexusClient;
use nexus::runtime::server::{self, ServerSecret};
use nexus::NexusService;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use stdio_mcp::StdioMcpConnection;
use tempfile::TempDir;
use tokio::task::JoinHandle;

pub const TEST_SECRET: &str = "correct-secret-that-is-at-least-32-bytes";
pub const TEST_PROJECT: &str = "shared-evidence-project";

pub struct RunningService {
    temporary: TempDir,
    address: SocketAddr,
    task: JoinHandle<()>,
}

impl RunningService {
    pub async fn start(allowed_origins: Vec<String>) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.storage.database_path = temporary.path().join("nexus.db");
        let service = Arc::new(
            NexusService::new(LoadedConfig {
                config,
                sources: Vec::new(),
                hash: "server-config".into(),
            })
            .unwrap(),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            server::serve_listener(
                listener,
                service,
                ServerSecret::new(TEST_SECRET).unwrap(),
                allowed_origins,
            )
            .await
            .unwrap();
        });
        Self {
            temporary,
            address,
            task,
        }
    }

    pub fn endpoint(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn client_config(&self, project_id: &str, reconcile_seconds: u64) -> LoadedConfig {
        let secret_file = self.temporary.path().join("client.secret");
        std::fs::write(&secret_file, TEST_SECRET).unwrap();
        std::fs::set_permissions(&secret_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut config = Config {
            client: ClientConfig::Remote(RemoteClientConfig {
                endpoint: format!("http://{}", self.address),
                project_id: project_id.into(),
                secret_file: Some(secret_file),
            }),
            ..Config::default()
        };
        config.coordination.reconcile_seconds = reconcile_seconds;
        LoadedConfig {
            config,
            sources: Vec::new(),
            hash: "client-config".into(),
        }
    }

    pub fn write_client_config(&self, project_id: &str, reconcile_seconds: u64) -> PathBuf {
        let loaded = self.client_config(project_id, reconcile_seconds);
        let path = self.temporary.path().join("remote-client.toml");
        std::fs::write(&path, toml::to_string_pretty(&loaded.config).unwrap()).unwrap();
        path
    }
}

impl Drop for RunningService {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug)]
pub struct CapturedRequest {
    pub authorization: Option<String>,
    pub body: Value,
}

pub struct CaptureEndpoint {
    pub address: SocketAddr,
    receiver: mpsc::Receiver<CapturedRequest>,
    task: JoinHandle<()>,
}

impl CaptureEndpoint {
    pub async fn start() -> Self {
        let (sender, receiver) = mpsc::channel();
        let app = Router::new()
            .route("/api/v1/request", post(capture_request))
            .with_state(sender);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            address,
            receiver,
            task,
        }
    }

    pub fn receive(&self) -> CapturedRequest {
        self.receiver.recv_timeout(Duration::from_secs(2)).unwrap()
    }
}

impl Drop for CaptureEndpoint {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn capture_request(
    State(sender): State<mpsc::Sender<CapturedRequest>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    sender
        .send(CapturedRequest {
            authorization,
            body,
        })
        .unwrap();
    Json(json!({"ok":true,"status":"running"}))
}

pub struct HttpMcpClient {
    client: reqwest::Client,
    endpoint: String,
    project_id: String,
    session_id: Option<String>,
}

impl HttpMcpClient {
    pub fn new(service: &RunningService, project_id: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: service.endpoint("/mcp"),
            project_id: project_id.into(),
            session_id: None,
        }
    }

    pub fn post_builder(
        &self,
        project_id: &str,
        session_id: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let request = self
            .client
            .post(&self.endpoint)
            .bearer_auth(TEST_SECRET)
            .header("accept", "application/json, text/event-stream")
            .header("x-nexus-project-id", project_id);
        match session_id {
            Some(session_id) => request
                .header("mcp-protocol-version", "2025-06-18")
                .header("mcp-session-id", session_id),
            None => request,
        }
    }

    async fn send_with(
        &self,
        project_id: &str,
        session_id: Option<&str>,
        payload: Value,
    ) -> reqwest::Response {
        self.post_builder(project_id, session_id)
            .json(&payload)
            .send()
            .await
            .unwrap()
    }

    pub async fn send(&self, payload: Value) -> reqwest::Response {
        self.send_with(&self.project_id, self.session_id.as_deref(), payload)
            .await
    }

    pub async fn send_as_project(&self, project_id: &str, payload: Value) -> reqwest::Response {
        self.send_with(project_id, self.session_id.as_deref(), payload)
            .await
    }

    pub async fn initialize(&mut self) -> Value {
        let response = self
            .send(json!({
                "jsonrpc":"2.0", "id":1, "method":"initialize",
                "params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}
            }))
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        self.session_id = Some(
            response
                .headers()
                .get("mcp-session-id")
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned(),
        );
        response.json().await.unwrap()
    }

    pub async fn notify_initialized(&self) {
        let response = self
            .send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    }

    pub async fn establish(&mut self) {
        self.initialize().await;
        self.notify_initialized().await;
    }

    pub async fn assert_session_header_rejections(&self) {
        let initialized = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        let missing = self.send_with("shared", None, initialized.clone()).await;
        assert_eq!(missing.status(), reqwest::StatusCode::BAD_REQUEST);
        let unknown = self
            .send_with("shared", Some("unknown-session"), initialized)
            .await;
        assert_eq!(unknown.status(), reqwest::StatusCode::NOT_FOUND);
    }

    pub async fn assert_get_semantics(&self) {
        let unauthenticated = self.client.get(&self.endpoint).send().await.unwrap();
        assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);
        let authenticated = self
            .client
            .get(&self.endpoint)
            .bearer_auth(TEST_SECRET)
            .header("origin", "https://agent.example.test")
            .header("mcp-protocol-version", "2025-06-18")
            .header("mcp-session-id", self.session_id.as_deref().unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(
            authenticated.status(),
            reqwest::StatusCode::METHOD_NOT_ALLOWED
        );
    }

    pub async fn assert_unsupported_transport_versions_are_rejected(&self) {
        let get = self
            .client
            .get(&self.endpoint)
            .bearer_auth(TEST_SECRET)
            .header("mcp-protocol-version", "1999-01-01")
            .header("mcp-session-id", self.session_id.as_deref().unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(get.status(), reqwest::StatusCode::BAD_REQUEST);

        let delete = self
            .client
            .delete(&self.endpoint)
            .bearer_auth(TEST_SECRET)
            .header("mcp-protocol-version", "1999-01-01")
            .header("mcp-session-id", self.session_id.as_deref().unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(delete.status(), reqwest::StatusCode::BAD_REQUEST);
    }

    pub async fn delete(&self) -> reqwest::Response {
        self.client
            .delete(&self.endpoint)
            .bearer_auth(TEST_SECRET)
            .header("mcp-protocol-version", "2025-06-18")
            .header("mcp-session-id", self.session_id.as_deref().unwrap())
            .send()
            .await
            .unwrap()
    }
}

pub struct StdioMcp {
    child: Child,
    connection: StdioMcpConnection,
}

impl StdioMcp {
    pub fn spawn(config: &Path, root: &Path) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nexus"));
        clear_git_environment(&mut command);
        clear_remote_client_environment(&mut command);
        let mut child = command
            .args(["--config", config.to_str().unwrap(), "mcp"])
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        Self {
            child,
            connection: StdioMcpConnection::new(input, output),
        }
    }

    pub fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.connection.call(name, arguments)
    }

    pub fn pre_tool(
        &mut self,
        root: &Path,
        session_id: &str,
        tool_use_id: &str,
        path: &str,
    ) -> Value {
        self.call(
            "nexus_pre_tool_use",
            json!({
                "session_id":session_id, "agent":"codex", "project_root":root,
                "tool_use_id":tool_use_id, "tool_name":"Write",
                "tool_input":{"file_path":path,"content":"replacement"}
            }),
        )
    }
}

impl Drop for StdioMcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn remote_client_config(
    address: SocketAddr,
    project_id: &str,
    secret_file: PathBuf,
) -> LoadedConfig {
    let config = Config {
        client: ClientConfig::Remote(RemoteClientConfig {
            endpoint: format!("http://{address}"),
            project_id: project_id.into(),
            secret_file: Some(secret_file),
        }),
        ..Config::default()
    };
    LoadedConfig {
        config,
        sources: Vec::new(),
        hash: "test-config".into(),
    }
}

pub fn initialized_checkout() -> TempDir {
    let checkout = tempfile::tempdir().unwrap();
    for arguments in [
        &["init"][..],
        &["config", "user.email", "nexus@example.test"],
        &["config", "user.name", "Nexus Test"],
    ] {
        run_git(checkout.path(), arguments);
    }
    std::fs::write(checkout.path().join("tracked.txt"), "initial\n").unwrap();
    run_git(checkout.path(), &["add", "tracked.txt"]);
    run_git(checkout.path(), &["commit", "-m", "initial"]);
    checkout
}

pub fn dirty_tracked_file(root: &Path) {
    std::fs::write(root.join("tracked.txt"), "changed outside hooks\n").unwrap();
}

pub fn install_usage_examples(root: &Path) -> PathBuf {
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::write(root.join("scripts/check.sh"), "#!/bin/sh\n").unwrap();
    let skill = root.join(".codex/skills/remote/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&skill, "---\nname: remote-evidence-skill\n---\n").unwrap();
    skill
}

pub fn completed_usage_requests(root: &Path, skill: &Path) -> Vec<ServiceRequest> {
    [
        json!({
            "session_id":"remote-evidence-session", "agent":"codex", "project_root":root,
            "tool_use_id":"shell-one", "tool_name":"Bash",
            "tool_input":{"command":"./scripts/check.sh"}
        }),
        json!({
            "session_id":"remote-evidence-session", "agent":"codex", "turn_id":"turn-one",
            "project_root":root, "tool_use_id":"read-skill", "tool_name":"Read",
            "tool_input":{"file_path":skill}
        }),
    ]
    .into_iter()
    .map(|params| ServiceRequest::decode("post_tool_use", params).unwrap())
    .collect()
}

pub async fn remote_claims(client: &NexusClient) -> Value {
    client
        .request(
            &ServiceRequest::decode("claims", json!({"project_id":TEST_PROJECT,"scope":"all"}))
                .unwrap(),
        )
        .await
        .unwrap()
}

pub async fn remote_usage(client: &NexusClient) -> Value {
    client
        .request(&ServiceRequest::decode("usage", json!({"project_id":TEST_PROJECT})).unwrap())
        .await
        .unwrap()
}

pub async fn wait_for_reconciled_claim(client: &NexusClient, session_id: &str, path: &str) {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let claims = remote_claims(client).await;
        let observed = claims["claims"].as_array().unwrap().iter().any(|claim| {
            claim["session_id"] == session_id
                && claim["path"] == path
                && claim["tool_use_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("git-reconcile:"))
        });
        if observed {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "remote companion did not reconcile {path} for {session_id}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn run_git(root: &Path, arguments: &[&str]) {
    let mut command = Command::new("git");
    clear_git_environment(&mut command);
    let output = command
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn clear_git_environment(command: &mut Command) {
    for variable in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
    ] {
        command.env_remove(variable);
    }
}

fn clear_remote_client_environment(command: &mut Command) {
    for variable in [
        "NEXUS_CONFIG",
        "NEXUS_ENDPOINT",
        "NEXUS_PROJECT_ID",
        "NEXUS_CLIENT_SECRET_FILE",
        "NEXUS_SECRET",
    ] {
        command.env_remove(variable);
    }
}
