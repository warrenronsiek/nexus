// @feature observability-ui
// @spec docs/features/observability-ui.md
// @entrypoint daemon_serves_read_only_observability_ui
// @boundary child-process-http
mod support;

use serde_json::json;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use support::{assert_success, git_command, wait_for_path};

#[test]
fn daemon_serves_read_only_observability_ui() {
    let temporary = tempfile::tempdir().unwrap();
    let address = unused_address();
    let config = temporary.path().join("config.toml");
    write_config(&config, temporary.path(), address);
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args(["--config", path(&config), "daemon"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_http(address, &mut daemon);

    let index = request(
        address,
        "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    assert!(lower(&index).starts_with("http/1.1 200"), "{index}");
    assert!(
        lower(&index).contains("content-security-policy:"),
        "{index}"
    );
    assert!(lower(&index).contains("cache-control: no-store"), "{index}");
    assert!(!lower(&index).contains("access-control-allow-origin"));
    assert!(index.contains("<div id=\"app\"></div>"), "{index}");

    for asset in ["/assets/elm.js", "/assets/app.js", "/assets/styles.css"] {
        let response = request(
            address,
            &format!("GET {asset} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"),
        );
        assert!(lower(&response).starts_with("http/1.1 200"), "{response}");
        assert!(response_body(&response).len() > 100, "{asset} was empty");
    }

    let health = request(
        address,
        "GET /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    let health_json = response_json(&health);
    assert_eq!(health_json["status"], "running");
    assert_eq!(health_json["mode"], "read_only");

    let dashboard = request(
        address,
        "GET /api/v1/dashboard HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    let dashboard_json = response_json(&dashboard);
    assert_eq!(dashboard_json["ok"], true);
    assert!(dashboard_json["events"]["items"].is_array());

    let rejected = request(
        address,
        "POST /api/v1/dashboard HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
    );
    assert!(lower(&rejected).starts_with("http/1.1 405"), "{rejected}");

    let foreign_host = request(
        address,
        "GET / HTTP/1.1\r\nHost: example.invalid\r\nConnection: close\r\n\r\n",
    );
    assert!(
        lower(&foreign_host).starts_with("http/1.1 403"),
        "{foreign_host}"
    );
    assert!(lower(&foreign_host).contains("content-security-policy:"));
    assert!(lower(&foreign_host).contains("cache-control: no-store"));

    let printed = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args(["--config", path(&config), "ui", "--launch", "print"])
        .output()
        .unwrap();
    assert_success("nexus ui", &printed);
    assert_eq!(
        String::from_utf8(printed.stdout).unwrap().trim(),
        format!("http://{address}")
    );

    daemon.kill().unwrap();
    daemon.wait().unwrap();
}

#[test]
fn occupied_ui_port_does_not_stop_coordination() {
    let temporary = tempfile::tempdir().unwrap();
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = occupied.local_addr().unwrap();
    let config = temporary.path().join("config.toml");
    write_config(&config, temporary.path(), address);
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args(["--config", path(&config), "daemon"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_path(&temporary.path().join("nexus.sock"), &mut daemon);

    let status = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args(["--config", path(&config), "status"])
        .output()
        .unwrap();
    assert_success("status with occupied UI port", &status);
    let status_json: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status_json["ok"], true);

    daemon.kill().unwrap();
    daemon.wait().unwrap();
}

#[tokio::test]
async fn dashboard_reports_conflicts_seeded_from_separate_worktrees() {
    let temporary = tempfile::tempdir().unwrap();
    let repository = temporary.path().join("repository");
    initialize_repository(&repository);
    let first = temporary.path().join("worktree-one");
    let second = temporary.path().join("worktree-two");
    run_git(&repository, &["worktree", "add", "--detach", path(&first)]);
    run_git(&repository, &["worktree", "add", "--detach", path(&second)]);

    let address = unused_address();
    let config = temporary.path().join("config.toml");
    write_config(&config, temporary.path(), address);
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .args(["--config", path(&config), "daemon"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let socket = temporary.path().join("nexus.sock");
    wait_for_path(&socket, &mut daemon);
    wait_for_http(address, &mut daemon);

    let project_id = record_prompt(&socket, "one", &first).await;
    record_prompt(&socket, "two", &second).await;
    record_edit(&socket, "one", &first).await;
    let second_edit = record_edit(&socket, "two", &second).await;
    assert_eq!(second_edit["advisories"].as_array().unwrap().len(), 1);

    let global = response_json(&request(
        address,
        "GET /api/v1/dashboard HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    ));
    let scoped = response_json(&request(
        address,
        &format!(
            "GET /api/v1/dashboard?project_id={project_id} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        ),
    ));
    assert_eq!(global["counts"]["open_conflicts"], 1);
    assert_eq!(scoped["counts"]["open_conflicts"], 1);
    assert_eq!(scoped["conflicts"]["items"][0]["kind"], "hunk_overlap");

    daemon.kill().unwrap();
    daemon.wait().unwrap();
}

async fn record_prompt(socket: &Path, session: &str, worktree: &Path) -> String {
    let request = nexus::coordination::api::ServiceRequest::decode(
        "user_prompt",
        json!({
            "session_id": session,
            "project_root": worktree,
            "agent": session,
            "prompt": "Edit shared.txt"
        }),
    )
    .unwrap();
    nexus::runtime::daemon::request(socket, &request)
        .await
        .unwrap()["project_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn record_edit(socket: &Path, session: &str, worktree: &Path) -> Value {
    let request = nexus::coordination::api::ServiceRequest::decode(
        "pre_tool_use",
        json!({
            "session_id": session,
            "project_root": worktree,
            "agent": session,
            "tool_use_id": format!("{session}-edit"),
            "tool_name": "Edit",
            "tool_input": {"file_path": "shared.txt", "line_start": 1, "line_end": 2}
        }),
    )
    .unwrap();
    nexus::runtime::daemon::request(socket, &request)
        .await
        .unwrap()
}

fn write_config(path: &Path, state: &Path, address: SocketAddr) {
    fs::write(
        path,
        format!(
            "schema_version = 1\n[storage]\ndatabase_path = {database:?}\n[runtime]\nsocket_path = {socket:?}\nlock_path = {lock:?}\n[ui]\nbind_address = \"{address}\"\n",
            database = state.join("nexus.db"),
            socket = state.join("nexus.sock"),
            lock = state.join("nexus.lock"),
        ),
    )
    .unwrap();
}

fn unused_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

fn wait_for_http(address: SocketAddr, daemon: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        assert!(daemon.try_wait().unwrap().is_none(), "daemon exited early");
        if TcpStream::connect(address).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("dashboard never became ready at {address}");
}

fn request(address: SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn lower(value: &str) -> String {
    value.to_ascii_lowercase()
}

fn response_json(response: &str) -> Value {
    let body = response_body(response);
    serde_json::from_str(body).unwrap()
}

fn response_body(response: &str) -> &str {
    response.split_once("\r\n\r\n").unwrap().1
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn initialize_repository(repository: &Path) {
    fs::create_dir_all(repository).unwrap();
    run_git(repository, &["init", "-b", "main"]);
    fs::write(repository.join("shared.txt"), "one\ntwo\nthree\n").unwrap();
    run_git(
        repository,
        &["config", "user.email", "nexus@example.invalid"],
    );
    run_git(repository, &["config", "user.name", "Nexus Test"]);
    run_git(repository, &["add", "shared.txt"]);
    run_git(repository, &["commit", "-m", "fixture"]);
}

fn run_git(directory: &Path, arguments: &[&str]) {
    let output = git_command()
        .args(arguments)
        .current_dir(directory)
        .output()
        .unwrap();
    assert_success("git", &output);
}
