// @feature runtime
// @feature work-history
// @spec docs/features/runtime.md
// @spec docs/features/work-history.md
// @boundary slow-delivery-fixture: exercise native process exit against delayed authenticated-work acknowledgments without rerunning a workload.
use super::support::remote_client_config;
use axum::{extract::State, routing::post, Json, Router};
use nexus::{runtime::work_capture::host_event, work_history::WorkAction};
use serde_json::{json, Value};
use std::{
    io::Write,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
struct AckDelays {
    work: Duration,
    lifecycle: Duration,
}

struct AckState {
    delays: Mutex<AckDelays>,
    received: Mutex<Vec<Value>>,
}

struct AckServer {
    state: Arc<AckState>,
    address: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl AckServer {
    async fn start(delays: AckDelays) -> Self {
        let state = Arc::new(AckState {
            delays: Mutex::new(delays),
            received: Mutex::new(Vec::new()),
        });
        let app = Router::new()
            .route("/api/v1/request", post(acknowledge))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            state,
            address,
            task,
        }
    }
}

impl Drop for AckServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn acknowledge(
    State(state): State<Arc<AckState>>,
    Json(request): Json<Value>,
) -> Json<Value> {
    let delays = *state.delays.lock().unwrap();
    if request["method"] == "work" {
        tokio::time::sleep(delays.work).await;
        state.received.lock().unwrap().push(request);
        Json(json!({"ok":true,"response":"recorded"}))
    } else {
        tokio::time::sleep(delays.lifecycle).await;
        Json(json!({"permitted":true,"recorded":true,"advisories":[]}))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_exit_delivers_retained_and_final_work_within_its_cleanup_budget() {
    let server = AckServer::start(AckDelays {
        work: Duration::from_millis(100),
        lifecycle: Duration::from_millis(350),
    })
    .await;
    let fixture = ExitFixture::new(server.address);
    fixture.seed();
    let output = fixture.stop().await;
    assert!(output.status.success(), "{output:?}");
    assert!(
        fixture.pending().is_empty(),
        "native exit left captures queued"
    );
    let received = server.state.received.lock().unwrap();
    assert!(received.len() >= 12, "retained records were not delivered");
    for request in received.iter() {
        for observation in request["params"]["command"]["observations"]
            .as_array()
            .unwrap()
        {
            if let Some(session) = observation["body"]["context"]["host_session_id"].as_str() {
                assert_eq!(
                    Some(session),
                    request["params"]["context"]["session_id"].as_str(),
                    "batch mixed caller identities"
                );
            }
        }
    }
    assert_eq!(
        finished_scopes(&received),
        std::collections::HashSet::from(["activity", "session"])
    );
}

fn finished_scopes(received: &[Value]) -> std::collections::HashSet<&str> {
    received
        .iter()
        .filter(|request| request["params"]["context"]["session_id"] == "departing")
        .flat_map(|request| {
            request["params"]["command"]["observations"]
                .as_array()
                .unwrap()
        })
        .filter(|observation| observation["body"]["event"] == "finished")
        .map(|observation| observation["subject"]["scope"].as_str().unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn final_activity_and_logical_session_finish_are_delivered_together() {
    let server = AckServer::start(AckDelays {
        work: Duration::ZERO,
        lifecycle: Duration::ZERO,
    })
    .await;
    let fixture = ExitFixture::new(server.address);
    let started = fixture.open().await;
    assert!(started.status.success(), "{started:?}");
    assert!(fixture.pending().is_empty());
    server.state.received.lock().unwrap().clear();
    *server.state.delays.lock().unwrap() = AckDelays {
        work: Duration::from_millis(400),
        lifecycle: Duration::from_millis(1850),
    };
    let output = fixture.stop().await;
    assert!(output.status.success(), "{output:?}");
    let received = server.state.received.lock().unwrap();
    assert_eq!(
        finished_scopes(&received),
        std::collections::HashSet::from(["activity", "session"]),
        "shutdown split the host and logical-session finish"
    );
    assert!(
        fixture.pending().is_empty(),
        "paired finish was not acknowledged"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_batch_retains_invalid_evidence_and_delivers_valid_neighbors() {
    let service = super::support::RunningService::start(Vec::new()).await;
    let fixture = ExitFixture::new(service.address());
    let loaded = service.client_config("exit-fixture", 3600);
    std::fs::write(&fixture.config, toml::to_string(&loaded.config).unwrap()).unwrap();
    fixture.seed_validation_batch();
    let mut command = fixture.nexus();
    command.args(["work", "flush"]);
    let output = tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let health: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(health["queued"], 0, "{health}");
    assert_eq!(health["rejected"], 1, "{health}");
    assert_eq!(health["recent_delivered"], 2, "{health}");
}

#[test]
fn unavailable_delivery_preserves_the_workload_and_queue_without_waiting_out_shutdown() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let fixture = ExitFixture::new(listener.local_addr().unwrap());
    drop(listener);
    fixture.seed();
    let started = Instant::now();
    let output = fixture
        .nexus()
        .args(["exec", "--", "sh", "-c", "printf executed; exit 7"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert_eq!(
        output.stdout, b"executed",
        "the workload must run exactly once"
    );
    assert!(
        !fixture.pending().is_empty(),
        "unacknowledged evidence must remain durable"
    );
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "unavailable delivery consumed the shutdown window: {:?}",
        started.elapsed()
    );
}

struct ExitFixture {
    temporary: tempfile::TempDir,
    config: PathBuf,
    queue: PathBuf,
}

impl ExitFixture {
    fn new(address: SocketAddr) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let secret = temporary.path().join("secret");
        std::fs::write(&secret, "fixture-secret-that-is-long-enough-for-auth").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        let loaded = remote_client_config(address, "exit-fixture", secret);
        let config = temporary.path().join("config.toml");
        std::fs::write(&config, toml::to_string(&loaded.config).unwrap()).unwrap();
        let queue = temporary.path().join("outbox");
        std::fs::create_dir(&queue).unwrap();
        std::fs::set_permissions(&queue, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            temporary,
            config,
            queue,
        }
    }

    fn seed(&self) {
        for index in 0..12 {
            let mut command = host_event(
                "codex",
                &serde_json::to_vec(
                    &json!({"session_id":format!("seed-{index}"),"hook_event_name":"SessionStart"}),
                )
                .unwrap(),
            )
            .unwrap();
            let WorkAction::Record { observations } = &mut command.command else {
                panic!("record")
            };
            observations.truncate(1);
            self.queue_command(&format!("seed-{index}"), &command);
        }
    }

    fn seed_validation_batch(&self) {
        for index in 0..3 {
            let mut command = host_event(
                "codex",
                b"{\"session_id\":\"departing\",\"hook_event_name\":\"SessionStart\"}",
            )
            .unwrap();
            let WorkAction::Record { observations } = &mut command.command else {
                panic!("record")
            };
            observations.truncate(1);
            observations[0].id = if index == 1 {
                String::new()
            } else {
                format!("valid-{index}")
            };
            observations[0].subject = nexus::work_history::WorkSubject::Session {
                id: format!("root-{index}"),
            };
            self.queue_command(&format!("entry-{index}"), &command);
        }
    }

    fn queue_command(&self, name: &str, command: &nexus::coordination::api::WorkCommand) {
        let path = self.queue.join(format!("{name}.json"));
        std::fs::write(&path, serde_json::to_vec(command).unwrap()).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    async fn open(&self) -> std::process::Output {
        let event = self.temporary.path().join("start.json");
        std::fs::write(
            &event,
            b"{\"session_id\":\"departing\",\"hook_event_name\":\"SessionStart\"}",
        )
        .unwrap();
        let mut command = self.nexus();
        command
            .args(["work", "host-event", "--host", "codex"])
            .arg(event);
        tokio::task::spawn_blocking(move || command.output().unwrap())
            .await
            .unwrap()
    }

    async fn stop(&self) -> std::process::Output {
        let mut command = self.nexus();
        tokio::task::spawn_blocking(move || {
            let mut child = command
                .args(["hook-session-end", "--agent", "codex"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"{\"session_id\":\"departing\"}")
                .unwrap();
            child.wait_with_output().unwrap()
        })
        .await
        .unwrap()
    }

    fn nexus(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nexus"));
        command
            .arg("--config")
            .arg(&self.config)
            .env("NEXUS_WORK_OUTBOX_DIR", &self.queue)
            .env_remove("NEXUS_WORK_CONTEXT")
            .env_remove("NEXUS_WORK_SESSION_ID")
            .env_remove("NEXUS_PARENT_ACTIVITY_ID")
            .env_remove("NEXUS_HOST_SESSION_ID")
            .env_remove("NEXUS_AGENT")
            .env_remove("NEXUS_SECRET")
            .current_dir(self.temporary.path());
        command
    }

    fn pending(&self) -> Vec<PathBuf> {
        std::fs::read_dir(&self.queue)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json" || extension == "callback")
            })
            .collect()
    }
}
