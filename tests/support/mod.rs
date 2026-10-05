// @feature coordination
// @feature persistence
// @feature runtime
// @feature analyst
// @feature installation
// @feature observability-ui
// @feature agent-memory
// @spec docs/features/coordination.md
// @spec docs/features/persistence.md
// @spec docs/features/runtime.md
// @spec docs/features/analyst.md
// @spec docs/features/installation.md
// @spec docs/features/observability-ui.md
// @spec docs/features/agent-memory.md
// @boundary dynamic-json
// Each integration-test crate uses a different subset of these shared helpers.
#![allow(dead_code)]

use nexus::persistence::Store;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

pub struct Harness {
    pub _temp: TempDir,
    pub root: PathBuf,
    pub database: PathBuf,
    pub daemon: Child,
    pub mcp: Child,
    pub input: ChildStdin,
    pub output: BufReader<ChildStdout>,
    pub next_id: u64,
}

struct HarnessOptions<'a> {
    analyst_command: Option<&'a Path>,
    codex_command: Option<&'a Path>,
    analyst_enabled: bool,
    claim_ttl_seconds: i64,
    consolidation_interval_seconds: u64,
    timeout_seconds: u64,
}

impl Harness {
    pub fn start() -> Self {
        Self::start_with_options(HarnessOptions {
            analyst_command: None,
            codex_command: None,
            analyst_enabled: false,
            claim_ttl_seconds: 120,
            consolidation_interval_seconds: 15,
            timeout_seconds: 60,
        })
    }

    pub fn start_with_claim_ttl(claim_ttl_seconds: i64) -> Self {
        Self::start_with_options(HarnessOptions {
            analyst_command: None,
            codex_command: None,
            analyst_enabled: false,
            claim_ttl_seconds,
            consolidation_interval_seconds: 15,
            timeout_seconds: 60,
        })
    }

    pub fn start_with_analyst(
        analyst_command: Option<&std::path::Path>,
        claim_ttl_seconds: i64,
    ) -> Self {
        Self::start_with_options(HarnessOptions {
            analyst_command,
            codex_command: None,
            analyst_enabled: analyst_command.is_some(),
            claim_ttl_seconds,
            consolidation_interval_seconds: 15,
            timeout_seconds: 60,
        })
    }

    pub fn start_with_memory_analysts(
        claude_command: &std::path::Path,
        codex_command: &std::path::Path,
        consolidation_interval_seconds: u64,
        timeout_seconds: u64,
    ) -> Self {
        Self::start_with_options(HarnessOptions {
            analyst_command: Some(claude_command),
            codex_command: Some(codex_command),
            analyst_enabled: false,
            claim_ttl_seconds: 120,
            consolidation_interval_seconds,
            timeout_seconds,
        })
    }

    fn start_with_options(options: HarnessOptions<'_>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let state = temp.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let database = state.join("nexus.db");
        let socket = state.join("nexus.sock");
        let config = temp.path().join("config.toml");
        std::fs::write(
            &config,
            test_config(&database, &socket, &state.join("nexus.lock"), &options),
        )
        .unwrap();

        let executable = env!("CARGO_BIN_EXE_nexus");
        let daemon = spawn_daemon(executable, &config);
        wait_for_socket(&socket);
        let (mcp, input, output) = spawn_mcp(executable, &config, &root);
        Self {
            _temp: temp,
            root,
            database,
            daemon,
            mcp,
            input,
            output,
            next_id: 1,
        }
    }

    pub fn initialize_git(&self) {
        run_git(&self.root, &["init"]);
        run_git(&self.root, &["config", "user.email", "nexus@example.test"]);
        run_git(&self.root, &["config", "user.name", "Nexus Test"]);
        std::fs::write(self.root.join("tracked.txt"), "initial\n").unwrap();
        run_git(&self.root, &["add", "tracked.txt"]);
        run_git(&self.root, &["commit", "-m", "initial"]);
    }

    pub fn wait_for_reconciled_claim(&mut self, session_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let response = self.call("nexus_claims", json!({"scope":"all"}));
            let observed = response["claims"]
                .as_array()
                .is_some_and(|claims| claims.iter().any(|claim| claim["session_id"] == session_id));
            if observed {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "reconciliation did not expose a claim through the daemon"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn observe_dirty_file(&mut self, session_id: &str) {
        let root = self.root.to_string_lossy().to_string();
        self.call(
            "nexus_user_prompt",
            json!({"session_id":session_id,"project_root":root,"agent":"codex","prompt":"Edit the tracked file"}),
        );
        std::fs::write(self.root.join("tracked.txt"), "changed outside hooks\n").unwrap();
        self.wait_for_reconciled_claim(session_id);
    }

    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], id);
        response
    }

    pub fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name":name,"arguments":arguments}))["result"]
            ["structuredContent"]
            .clone()
    }

    pub fn status(&mut self) -> Value {
        self.call("nexus_status", json!({}))
    }

    pub fn hook(
        &mut self,
        name: &str,
        session: &str,
        tool_id: &str,
        tool_name: &str,
        tool_input: Value,
    ) -> Value {
        let root = self.root.to_string_lossy().to_string();
        self.call(
            name,
            json!({
                "session_id":session,
                "project_root":root,
                "agent":if session.starts_with("codex") { "codex" } else { "claude" },
                "tool_use_id":tool_id,
                "tool_name":tool_name,
                "tool_input":tool_input
            }),
        )
    }

    pub fn store(&self) -> Store {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match Store::open(&self.database) {
                Ok(store) => return store,
                // A running daemon can briefly hold SQLite's write lock while this
                // independent test reader repeats migration/schema checks.
                Err(_error) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => panic!("open test store: {error:#}"),
            }
        }
    }

    pub fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.input,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
        id
    }

    pub fn post_tool_arguments(&self, session: &str, tool_id: &str) -> Value {
        json!({
            "session_id":session,"project_root":self.root.to_string_lossy(),"agent":"codex",
            "tool_use_id":tool_id,"tool_name":"exec_command","tool_input":{"cmd":"true"}
        })
    }

    /// Reads the next MCP response on a worker thread so a hung adapter fails the test
    /// instead of hanging it.
    pub fn next_response_within(mut self, budget: Duration) -> (Self, Value) {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            self.output.read_line(&mut line).unwrap();
            sender
                .send((self, serde_json::from_str::<Value>(&line).unwrap()))
                .ok();
        });
        receiver
            .recv_timeout(budget)
            .expect("MCP lifecycle response exceeded the host hook budget")
    }
}

fn wait_for_socket(socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !socket.exists() {
        assert!(Instant::now() < deadline, "daemon socket did not appear");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn spawn_mcp(
    executable: &str,
    config: &Path,
    root: &Path,
) -> (Child, ChildStdin, BufReader<ChildStdout>) {
    let mut child = Command::new(executable)
        .args(["--config", config.to_str().unwrap(), "mcp"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let output = BufReader::new(child.stdout.take().unwrap());
    (child, input, output)
}

/// Stays shorter than the five-second timeout generated for host lifecycle hooks.
pub const HOST_HOOK_BUDGET: Duration = Duration::from_millis(4_500);

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.mcp.kill();
        let _ = self.mcp.wait();
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

fn spawn_daemon(executable: &str, config: &Path) -> Child {
    let mut command = Command::new(executable);
    clear_git_environment(&mut command);
    command
        .args(["--config", config.to_str().unwrap(), "daemon"])
        // Prove synchronous service work cannot starve the async runtime.
        .env("TOKIO_WORKER_THREADS", "1");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn test_config(
    database: &std::path::Path,
    socket: &std::path::Path,
    lock: &std::path::Path,
    options: &HarnessOptions<'_>,
) -> String {
    let reconcile_seconds = if options.analyst_command.is_some() {
        30
    } else {
        1
    };
    let analyst_command = options
        .analyst_command
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "claude".into());
    let codex_command = options
        .codex_command
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "codex".into());
    let HarnessOptions {
        analyst_enabled,
        claim_ttl_seconds,
        consolidation_interval_seconds,
        timeout_seconds,
        ..
    } = options;
    format!(
        r#"schema_version = 1
[coordination]
reconcile_seconds = {reconcile_seconds}
claim_ttl_seconds = {claim_ttl_seconds}
[storage]
database_path = '{}'
[runtime]
socket_path = '{}'
lock_path = '{}'
[analyst]
enabled = {analyst_enabled}
provider = "claude"
timeout_seconds = {timeout_seconds}
[analyst.codex]
command = '{}'
[analyst.claude]
command = '{}'
[memory]
consolidation_interval_seconds = {consolidation_interval_seconds}
"#,
        database.display(),
        socket.display(),
        lock.display(),
        codex_command,
        analyst_command
    )
}

pub fn run_git(root: &std::path::Path, arguments: &[&str]) {
    let output = git_command()
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn wait_for_path(target: &Path, daemon: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        assert!(daemon.try_wait().unwrap().is_none(), "daemon exited early");
        if target.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("path never became ready at {}", target.display());
}

pub fn assert_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn write_executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

pub fn git_command() -> Command {
    let mut command = Command::new("git");
    clear_git_environment(&mut command);
    command
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
