// @feature runtime
// @feature configuration
// @feature coordination
// @feature usage-analytics
// @feature sos
// @feature observability-ui
// @spec docs/features/runtime.md
// @spec docs/features/configuration.md
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/sos.md
// @spec docs/features/observability-ui.md
// @boundary dynamic-json
#[path = "remote_runtime/lifecycle_exit.rs"]
mod lifecycle_exit;
#[path = "remote_runtime/lifecycle_retry.rs"]
mod lifecycle_retry;
#[path = "remote_runtime/support.rs"]
mod support;

use nexus::coordination::api::ServiceRequest;
use nexus::runtime::client::NexusClient;
use nexus::runtime::server;
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;
use support::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_terminal_works_without_a_local_database_directory() {
    let service = RunningService::start(Vec::new()).await;
    let temporary = tempfile::tempdir().unwrap();
    let unavailable = temporary.path().join("legacy state");
    std::fs::write(&unavailable, "archived local state").unwrap();
    let mut loaded = service.client_config("terminal-project", 3600);
    loaded.config.storage.database_path = unavailable.join("nexus.db");
    let config = temporary.path().join("remote terminal.toml");
    std::fs::write(&config, toml::to_string_pretty(&loaded.config).unwrap()).unwrap();

    let output = std::process::Command::new("python3")
        .args([
            "tests/tui_terminal.py",
            env!("CARGO_BIN_EXE_nexus"),
            "system",
            "quit",
        ])
        .arg(&config)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(unavailable).unwrap(),
        "archived local state"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_terminal_opens_before_slow_reads_and_recovers_after_timeout() {
    let service = RunningService::start(Vec::new()).await;
    let config = service.write_client_config("terminal-recovery", 3600);
    let output = std::process::Command::new("python3")
        .args(["tests/tui_startup.py", env!("CARGO_BIN_EXE_nexus")])
        .arg(config)
        .arg(service.endpoint(""))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_client_authenticates_and_sends_project_outside_feature_payload() {
    let endpoint = CaptureEndpoint::start().await;
    let temporary = tempfile::tempdir().unwrap();
    let secret_file = temporary.path().join("secret");
    std::fs::write(&secret_file, "test-secret-that-is-long-enough-for-auth\r\n").unwrap();
    std::fs::set_permissions(&secret_file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let loaded = remote_client_config(endpoint.address, "shared-project", secret_file);

    let response = NexusClient::new(&loaded, None)
        .request(&ServiceRequest::Status)
        .await
        .unwrap();
    assert_eq!(response["status"], "running");
    let captured = endpoint.receive();
    assert_eq!(
        captured.authorization.as_deref(),
        Some("Bearer test-secret-that-is-long-enough-for-auth")
    );
    assert_eq!(captured.body["project_id"], "shared-project");
    assert_eq!(captured.body["method"], "status");
    assert_eq!(captured.body["params"], json!({}));
}

#[tokio::test]
async fn remote_companion_rejects_an_overexposed_secret_file() {
    let endpoint = CaptureEndpoint::start().await;
    let temporary = tempfile::tempdir().unwrap();
    let secret_file = temporary.path().join("secret");
    std::fs::write(&secret_file, "test-secret-that-is-long-enough-for-auth").unwrap();
    std::fs::set_permissions(&secret_file, std::fs::Permissions::from_mode(0o644)).unwrap();
    let loaded = remote_client_config(endpoint.address, "shared-project", secret_file);

    let error = NexusClient::new(&loaded, None)
        .request(&ServiceRequest::Status)
        .await
        .expect_err("the companion must reject a bearer secret visible to other users");

    assert!(
        format!("{error:#}").contains("permissions 644"),
        "{error:#}"
    );
}

#[tokio::test]
async fn remote_service_rejects_bad_bearer_before_dispatch() {
    let service = RunningService::start(Vec::new()).await;
    let client = reqwest::Client::new();
    let endpoint = service.endpoint("/api/v1/request");
    let payload = json!({"method":"status","params":{},"project_id":"shared"});

    let rejected = client
        .post(&endpoint)
        .bearer_auth("wrong-secret-that-is-also-long-enough")
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), reqwest::StatusCode::UNAUTHORIZED);

    let accepted = client
        .post(endpoint)
        .bearer_auth(TEST_SECRET)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), reqwest::StatusCode::OK);
    assert_eq!(accepted.json::<Value>().await.unwrap()["status"], "running");
}

#[tokio::test]
async fn direct_sos_ingestion_authenticates_and_binds_the_header_project() {
    let service = RunningService::start(Vec::new()).await;
    let client = reqwest::Client::new();
    let endpoint = service.endpoint("/api/v1/sos");
    let payload = direct_sos_payload();

    assert_direct_sos_admission_rejections(&client, &endpoint, &payload).await;
    assert_direct_sos_reporters(&client, &endpoint, &payload).await;
    assert_sos_untriaged_count(&client, &service, 2).await;
}

fn direct_sos_payload() -> Value {
    json!({
        "message":"Release pipeline is wedged",
        "type":"build_failure",
        "environment":"ci-linux",
        "version":"1.7.0",
        "model":"gpt-6",
        "trace":"step=publish\nexit=137",
        "metadata":{"job":"release","attempt":3}
    })
}

async fn direct_sos_request(
    client: &reqwest::Client,
    endpoint: &str,
    payload: &Value,
    authenticated: bool,
    agent: Option<&str>,
) -> reqwest::Response {
    let mut request = client
        .post(endpoint)
        .header("x-nexus-project-id", "sos-project")
        .json(payload);
    if authenticated {
        request = request.bearer_auth(TEST_SECRET);
    }
    if let Some(agent) = agent {
        request = request.header("x-nexus-agent", agent);
    }
    request.send().await.unwrap()
}

async fn assert_direct_sos_admission_rejections(
    client: &reqwest::Client,
    endpoint: &str,
    payload: &Value,
) {
    let rejected = direct_sos_request(client, endpoint, payload, false, None).await;
    assert_eq!(rejected.status(), reqwest::StatusCode::UNAUTHORIZED);

    let mut spoofed_project = payload.clone();
    spoofed_project["project_id"] = json!("other-project");
    let invalid_body = direct_sos_request(client, endpoint, &spoofed_project, true, None).await;
    assert_eq!(invalid_body.status(), reqwest::StatusCode::BAD_REQUEST);

    let oversized_agent = "a".repeat(257);
    let invalid_agent =
        direct_sos_request(client, endpoint, payload, true, Some(&oversized_agent)).await;
    assert_eq!(invalid_agent.status(), reqwest::StatusCode::BAD_REQUEST);
}

async fn assert_direct_sos_reporters(client: &reqwest::Client, endpoint: &str, payload: &Value) {
    let accepted = direct_sos_request(client, endpoint, payload, true, Some("incident-bot")).await;
    assert_eq!(accepted.status(), reqwest::StatusCode::ACCEPTED);
    let reported = accepted.json::<Value>().await.unwrap();
    assert_eq!(reported["ok"], true, "{reported}");
    assert_eq!(reported["report"]["message"], payload["message"]);
    assert_eq!(reported["report"]["type"], payload["type"]);
    assert_eq!(reported["report"]["metadata"], payload["metadata"]);
    assert_eq!(reported["report"]["reporter"]["agent"], "incident-bot");

    let default_agent = direct_sos_request(client, endpoint, payload, true, None).await;
    assert_eq!(default_agent.status(), reqwest::StatusCode::ACCEPTED);
    assert_eq!(
        default_agent.json::<Value>().await.unwrap()["report"]["reporter"]["agent"],
        "sos-http"
    );
}

async fn assert_sos_untriaged_count(
    client: &reqwest::Client,
    service: &RunningService,
    expected: u64,
) {
    let listed = client
        .post(service.endpoint("/api/v1/request"))
        .bearer_auth(TEST_SECRET)
        .json(&json!({
            "method":"sos",
            "project_id":"sos-project",
            "params":{"action":"list","project_root":"/ignored","session_id":"reader","agent":"test"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), reqwest::StatusCode::OK);
    let listed = listed.json::<Value>().await.unwrap();
    assert_eq!(listed["untriaged_report_count"], expected);
}

#[tokio::test]
async fn remote_reconciliation_rejects_identities_outside_projection_bounds() {
    let service = RunningService::start(Vec::new()).await;
    let endpoint = service.endpoint("/api/v1/request");
    let client = reqwest::Client::new();

    for (session_id, agent) in [
        ("session\0suffix".to_owned(), "codex".to_owned()),
        ("session".to_owned(), "a".repeat(257)),
    ] {
        let response = client
            .post(&endpoint)
            .bearer_auth(TEST_SECRET)
            .json(&json!({
                "method": "client_reconcile",
                "params": {},
                "project_id": TEST_PROJECT,
                "client_evidence": {
                    "session_id": session_id,
                    "agent": agent,
                    "changed_paths": [],
                    "changed_paths_truncated": false
                }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn remote_service_rejects_bad_bearer_before_reading_the_body() {
    let service = RunningService::start(Vec::new()).await;
    let mut connection = tokio::net::TcpStream::connect(service.address())
        .await
        .unwrap();
    connection
        .write_all(
            b"POST /api/v1/request HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer wrong-secret-that-is-also-long-enough\r\nContent-Type: application/json\r\nContent-Length: 1048576\r\n\r\n{",
        )
        .await
        .unwrap();

    let mut response = [0_u8; 512];
    let read = tokio::time::timeout(Duration::from_millis(500), connection.read(&mut response))
        .await
        .expect("authentication must reject before the request body completes")
        .unwrap();
    let response = String::from_utf8_lossy(&response[..read]);
    assert!(
        response.starts_with("HTTP/1.1 401 Unauthorized"),
        "{response}"
    );
}

#[tokio::test]
async fn remote_service_rejects_oversized_authenticated_bodies() {
    let service = RunningService::start(Vec::new()).await;
    let response = reqwest::Client::new()
        .post(service.endpoint("/api/v1/request"))
        .bearer_auth(TEST_SECRET)
        .header("content-type", "application/json")
        .body(vec![b'x'; 1024 * 1024 + 1])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn authenticated_peer_cannot_stall_a_request_body_forever() {
    let service = RunningService::start(Vec::new()).await;
    let mut connection = tokio::net::TcpStream::connect(service.address())
        .await
        .unwrap();
    let request = format!(
        "POST /api/v1/request HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TEST_SECRET}\r\nContent-Type: application/json\r\nContent-Length: 1048576\r\n\r\n{{"
    );
    connection.write_all(request.as_bytes()).await.unwrap();

    let mut response = [0_u8; 512];
    let read = tokio::time::timeout(Duration::from_secs(7), connection.read(&mut response))
        .await
        .expect("bounded request body read must complete")
        .unwrap();
    let response = String::from_utf8_lossy(&response[..read]);
    assert!(
        response.starts_with("HTTP/1.1 408 Request Timeout"),
        "{response}"
    );
}

#[tokio::test]
async fn streamable_http_mcp_requires_json_and_initialization() {
    let service = RunningService::start(vec!["https://agent.example.test".into()]).await;
    let mut mcp = HttpMcpClient::new(&service, "shared");
    let wrong_content_type = mcp
        .post_builder("shared", None)
        .body(r#"{"jsonrpc":"2.0","id":0,"method":"initialize"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(
        wrong_content_type.status(),
        reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE
    );

    let initialized = mcp.initialize().await;
    assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
    let premature = mcp
        .send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .await;
    assert_eq!(premature.status(), reqwest::StatusCode::BAD_REQUEST);

    mcp.notify_initialized().await;
    let tools = mcp
        .send(json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}))
        .await;
    assert_eq!(tools.status(), reqwest::StatusCode::OK);
    assert!(tools.json::<Value>().await.unwrap()["result"]["tools"]
        .as_array()
        .is_some_and(|tools| !tools.is_empty()));
}

#[tokio::test]
async fn streamable_http_mcp_binds_and_tears_down_sessions() {
    let service = RunningService::start(vec!["https://agent.example.test".into()]).await;
    let mut mcp = HttpMcpClient::new(&service, "shared");
    mcp.establish().await;

    let changed_project = mcp
        .send_as_project(
            "different-project",
            json!({"jsonrpc":"2.0","id":4,"method":"tools/list","params":{}}),
        )
        .await;
    assert_eq!(changed_project.status(), reqwest::StatusCode::BAD_REQUEST);
    mcp.assert_session_header_rejections().await;
    mcp.assert_get_semantics().await;
    mcp.assert_unsupported_transport_versions_are_rejected()
        .await;

    assert_eq!(mcp.delete().await.status(), reqwest::StatusCode::NO_CONTENT);
    let after_delete = mcp
        .send(json!({"jsonrpc":"2.0","id":5,"method":"tools/list","params":{}}))
        .await;
    assert_eq!(after_delete.status(), reqwest::StatusCode::NOT_FOUND);
}

#[test]
fn generated_server_secret_is_persisted_once_with_private_permissions() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("credentials/service.secret");

    let first = server::load_or_generate_secret(None, Some(&path)).unwrap();
    assert!(first.was_generated());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let generated = std::fs::read_to_string(&path).unwrap();
    assert!(generated.trim().len() >= 64);

    let second = server::load_or_generate_secret(None, Some(&path)).unwrap();
    assert!(!second.was_generated());
    assert_eq!(std::fs::read_to_string(path).unwrap(), generated);
    assert!(server::load_or_generate_secret(None, None).is_err());
}

#[test]
fn existing_server_secret_rejects_permissions_broader_than_owner_read_write() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("service.secret");
    std::fs::write(&path, "test-secret-that-is-long-enough-for-auth").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();

    let error = server::load_or_generate_secret(None, Some(&path))
        .err()
        .expect("permissive secret file must be rejected");

    assert!(format!("{error:#}").contains("permissions"), "{error:#}");
}

#[test]
fn existing_server_secret_rejects_symbolic_links() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("target.secret");
    let path = temporary.path().join("service.secret");
    std::fs::write(&target, "test-secret-that-is-long-enough-for-auth").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();

    let error = server::load_or_generate_secret(None, Some(&path))
        .err()
        .expect("symbolic-link secret file must be rejected");

    assert!(error.to_string().contains("symbolic link"), "{error:#}");
}

#[test]
fn existing_server_secret_rejects_non_regular_files() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("service.secret");
    std::fs::create_dir(&path).unwrap();

    let error = server::load_or_generate_secret(None, Some(&path))
        .err()
        .expect("non-regular secret file must be rejected");

    assert!(format!("{error:#}").contains("regular file"), "{error:#}");
}

#[test]
fn existing_server_secret_rejects_fifo_and_oversized_files_without_blocking() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::time::{Duration, Instant};

    let temporary = tempfile::tempdir().unwrap();
    let fifo = temporary.path().join("service.fifo");
    let fifo_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);
    let started = Instant::now();
    let error = server::load_or_generate_secret(None, Some(&fifo))
        .err()
        .expect("FIFO secret file must be rejected");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(format!("{error:#}").contains("regular file"), "{error:#}");

    let oversized = temporary.path().join("oversized.secret");
    std::fs::write(&oversized, "x".repeat(4 * 1024 + 3)).unwrap();
    std::fs::set_permissions(&oversized, std::fs::Permissions::from_mode(0o600)).unwrap();
    let error = server::load_or_generate_secret(None, Some(&oversized))
        .err()
        .expect("oversized secret file must be rejected");
    assert!(format!("{error:#}").contains("exceeds"), "{error:#}");

    let unpublished = temporary.path().join("unpublished.secret");
    let error =
        server::load_or_generate_secret(Some(&"x".repeat(4 * 1024 + 1)), Some(&unpublished))
            .err()
            .expect("oversized explicit secret must be rejected before persistence");
    assert!(error.to_string().contains("must not exceed"), "{error:#}");
    assert!(!unpublished.exists());
}

#[test]
fn maximum_length_explicit_secret_round_trips_through_persistence() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("service.secret");
    let secret = "x".repeat(4 * 1024);

    server::load_or_generate_secret(Some(&secret), Some(&path))
        .expect("the maximum-length secret must be persisted");
    assert_eq!(std::fs::read(&path).unwrap().len(), secret.len());
    server::load_or_generate_secret(None, Some(&path))
        .expect("the persisted maximum-length secret must reload");
}

#[test]
fn maximum_length_newline_terminated_secret_file_is_accepted() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("service.secret");
    std::fs::write(&path, format!("{}\r\n", "x".repeat(4 * 1024))).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    server::load_or_generate_secret(None, Some(&path))
        .expect("the canonical maximum must allow a conventional line ending");
}

#[test]
fn concurrent_secret_initialization_never_replaces_the_winner() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("credentials/service.secret");
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let workers = (0..8)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                server::load_or_generate_secret(None, Some(&path))
                    .unwrap()
                    .was_generated()
            })
        })
        .collect::<Vec<_>>();
    let generated_count = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .filter(|generated| *generated)
        .count();

    assert_eq!(generated_count, 1);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(std::fs::read_to_string(path).unwrap().trim().len(), 64);
}

#[tokio::test]
async fn remote_companion_forwards_derived_usage_evidence_after_completed_tools() {
    let checkout = initialized_checkout();
    let skill = install_usage_examples(checkout.path());
    let service = RunningService::start(Vec::new()).await;
    let loaded = service.client_config(" shared-evidence-project ", 30);
    let client = NexusClient::new(&loaded, None);

    for request in completed_usage_requests(checkout.path(), &skill) {
        assert_eq!(client.lifecycle_request(&request).await["permitted"], true);
    }

    let usage = remote_usage(&client).await;
    assert!(usage["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|count| count["kind"] == "script" && count["name"] == "scripts/check.sh"));
    assert!(usage["skills"]
        .as_array()
        .unwrap()
        .iter()
        .any(|count| count["name"] == "remote-evidence-skill"));
}

#[tokio::test]
async fn invalid_inferred_usage_does_not_drop_the_completed_tool_event() {
    let checkout = initialized_checkout();
    let skill = checkout.path().join(".codex/skills/oversized/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&skill, format!("---\nname: {}\n---\n", "x".repeat(513))).unwrap();
    let request = completed_usage_requests(checkout.path(), &skill)
        .pop()
        .unwrap();
    let script = format!(
        "scripts/{}/{}/{}.sh",
        "a".repeat(200),
        "b".repeat(200),
        "c".repeat(120)
    );
    let script_path = checkout.path().join(&script);
    std::fs::create_dir_all(script_path.parent().unwrap()).unwrap();
    std::fs::write(&script_path, "#!/bin/sh\n").unwrap();
    let script_request = ServiceRequest::decode(
        "post_tool_use",
        json!({
            "session_id":"remote-evidence-session", "agent":"codex",
            "project_root":checkout.path(), "tool_use_id":"oversized-script",
            "tool_name":"Bash", "tool_input":{"command":format!("./{script}")}
        }),
    )
    .unwrap();
    let service = RunningService::start(Vec::new()).await;
    let client = NexusClient::new(&service.client_config(TEST_PROJECT, 30), None);

    for request in [request, script_request] {
        assert_eq!(client.lifecycle_request(&request).await["permitted"], true);
    }
    let usage = remote_usage(&client).await;
    let tools = usage["tools"].as_array().unwrap();
    assert!(tools
        .iter()
        .any(|count| count["kind"] == "tool" && count["name"] == "read"));
    assert!(tools
        .iter()
        .any(|count| count["kind"] == "tool" && count["name"] == "shell"));
    assert!(usage["skills"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_stdio_companion_reconciles_only_real_lifecycle_sessions() {
    let checkout = initialized_checkout();
    dirty_tracked_file(checkout.path());
    let service = RunningService::start(Vec::new()).await;
    let loaded = service.client_config(TEST_PROJECT, 1);
    let client = NexusClient::new(&loaded, None);
    let config = service.write_client_config(TEST_PROJECT, 1);
    let mut mcp = StdioMcp::spawn(&config, checkout.path());

    assert!(mcp.call("nexus_claims", json!({"scope":"all"}))["claims"]
        .as_array()
        .unwrap()
        .is_empty());
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert!(remote_claims(&client).await["claims"]
        .as_array()
        .unwrap()
        .is_empty());

    let first = mcp.pre_tool(checkout.path(), "session-one", "unrelated", "README.md");
    assert!(first["advisories"].as_array().unwrap().is_empty());
    wait_for_reconciled_claim(&client, "session-one", "tracked.txt").await;

    let same_session = mcp.pre_tool(checkout.path(), "session-one", "same", "tracked.txt");
    assert!(same_session["advisories"].as_array().unwrap().is_empty());
    let second_session = mcp.pre_tool(checkout.path(), "session-two", "other", "tracked.txt");
    assert_eq!(
        second_session["advisories"][0]["other_session_id"],
        "session-one"
    );
    assert_eq!(second_session["advisories"][0]["kind"], "path_overlap");
}

#[path = "remote_runtime/cross_checkout.rs"]
mod cross_checkout;
