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

async fn acknowledge(
    State(received): State<Arc<Mutex<Vec<Value>>>>,
    Json(request): Json<Value>,
) -> Json<Value> {
    if request["method"] == "work" {
        tokio::time::sleep(Duration::from_millis(100)).await;
        received.lock().unwrap().push(request);
        Json(json!({"ok":true,"response":"recorded"}))
    } else {
        // Allow the initial periodic drain to finish before the stop response.
        tokio::time::sleep(Duration::from_millis(350)).await;
        Json(json!({"permitted":true,"recorded":true,"advisories":[]}))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_exit_delivers_retained_and_final_work_within_its_cleanup_budget() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/api/v1/request", post(acknowledge))
        .with_state(received.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let fixture = ExitFixture::new(address);
    fixture.seed();
    let output = fixture.stop().await;
    assert!(output.status.success(), "{output:?}");
    assert!(
        fixture.pending().is_empty(),
        "native exit left captures queued"
    );
    let received = received.lock().unwrap();
    assert!(received.len() >= 12, "retained records were not delivered");
    let scopes: std::collections::HashSet<_> = received
        .iter()
        .filter(|request| request["params"]["context"]["session_id"] == "departing")
        .flat_map(|request| {
            request["params"]["command"]["observations"]
                .as_array()
                .unwrap()
        })
        .filter(|observation| observation["body"]["event"] == "finished")
        .map(|observation| observation["subject"]["scope"].as_str().unwrap())
        .collect();
    assert_eq!(
        scopes,
        std::collections::HashSet::from(["activity", "session"])
    );
    server.abort();
}

#[test]
fn unavailable_delivery_preserves_the_workload_and_queue_without_waiting_out_shutdown() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let fixture = ExitFixture::new(listener.local_addr().unwrap());
    drop(listener);
    fixture.seed();
    let started = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_nexus"))
        .arg("--config")
        .arg(&fixture.config)
        .args(["exec", "--", "sh", "-c", "printf executed; exit 7"])
        .env("NEXUS_WORK_OUTBOX_DIR", &fixture.queue)
        .env_remove("NEXUS_WORK_CONTEXT")
        .env_remove("NEXUS_WORK_SESSION_ID")
        .env_remove("NEXUS_PARENT_ACTIVITY_ID")
        .env_remove("NEXUS_HOST_SESSION_ID")
        .current_dir(fixture.temporary.path())
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
            std::fs::write(
                self.queue.join(format!("seed-{index}.json")),
                serde_json::to_vec(&command).unwrap(),
            )
            .unwrap();
            std::fs::set_permissions(
                self.queue.join(format!("seed-{index}.json")),
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
    }

    async fn stop(&self) -> std::process::Output {
        let working_directory = self.temporary.path().to_path_buf();
        let config = self.config.clone();
        let queue = self.queue.clone();
        tokio::task::spawn_blocking(move || {
            let mut child = Command::new(env!("CARGO_BIN_EXE_nexus"))
                .args(["--config"])
                .arg(config)
                .args(["hook-session-end", "--agent", "codex"])
                .env("NEXUS_WORK_OUTBOX_DIR", queue)
                .env_remove("NEXUS_WORK_CONTEXT")
                .env_remove("NEXUS_WORK_SESSION_ID")
                .env_remove("NEXUS_PARENT_ACTIVITY_ID")
                .env_remove("NEXUS_HOST_SESSION_ID")
                .current_dir(working_directory)
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
