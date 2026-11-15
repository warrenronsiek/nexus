// @feature delivery-coordination
// @feature runtime
// @spec docs/features/delivery-coordination.md
// @spec docs/features/runtime.md
// @boundary daemon-provider-fixture: a local HTTP server and isolated daemon verify autonomous observation without live provider credentials.
#![cfg(feature = "sqlite")]
use nexus::{
    delivery::{DeliveryAction, DeliveryResponse},
    persistence::Store,
    work_history::WorkActor,
};
use serde_json::json;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct DaemonFixture {
    _directory: tempfile::TempDir,
    database: PathBuf,
    config: PathBuf,
    child: Option<Child>,
}
impl DaemonFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("watch.db");
        let path = directory.path().join("nexus.toml");
        let mut config = nexus::Config::default();
        config.storage.database_path = database.clone();
        config.runtime.socket_path = directory.path().join("nexus.sock");
        config.runtime.lock_path = directory.path().join("nexus.lock");
        config.analyst.enabled = false;
        config.analytics.classification_enabled = false;
        std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
        Self {
            _directory: directory,
            database,
            config: path,
            child: None,
        }
    }
    fn seed(&self) {
        let mut store = Store::open(&self.database).unwrap();
        let revision = |sha: char| json!({"repository":"repo","object_format":"sha1","commit":sha.to_string().repeat(40)});
        let events = [
            json!({"event":"pull_request_registered","pull_request":{"id":"pr","repository":"repo","provider":"github","number":42,"url":"https://github.com/example/repo/pull/42","title":"Release","base":revision('a'),"head":revision('b'),"contribution_ids":[]}}),
            json!({"event":"watch_registered","watch":{"id":"watch","pull_request_id":"pr","repository_id":"repo","source":{"owner":"example","repository":"repo","number":42},"interval_seconds":30}}),
        ];
        let observations = events.into_iter().enumerate().map(|(n,body)| json!({"id":format!("seed-{n}"),"occurred_at":"2026-11-15T12:00:00Z","body":body})).collect::<Vec<_>>();
        let action =
            serde_json::from_value(json!({"action":"record","observations":observations})).unwrap();
        store
            .delivery_command("project", &actor(), &action)
            .unwrap();
    }
    fn start(&mut self, api_base: &str) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nexus"));
        command
            .args(["--config", self.config.to_str().unwrap(), "daemon"])
            .current_dir(self._directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (key, _) in
            std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("NEXUS_"))
        {
            command.env_remove(key);
        }
        command.env("NEXUS_DELIVERY_GITHUB_API_URL", api_base);
        self.child = Some(command.spawn().unwrap());
    }
    async fn await_refresh(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut store = Store::open(&self.database).unwrap();
        loop {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "daemon exited before refreshing watch"
            );
            let response = store
                .delivery_command(
                    "project",
                    &actor(),
                    &DeliveryAction::Watches { project_id: None },
                )
                .unwrap();
            if let DeliveryResponse::Watches { watches, .. } = response {
                if watches[0].last_success_at.is_some() {
                    assert!(watches[0].last_error.is_none());
                    verify_snapshot(&mut store);
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "watch never refreshed without an agent"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    async fn stop(&mut self) {
        let child = self.child.as_mut().unwrap();
        // SAFETY: this is the fixture-owned child process, still running above.
        unsafe {
            libc::kill(child.id() as i32, libc::SIGINT);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if child.try_wait().unwrap().is_some() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "watcher prevented daemon shutdown"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
impl Drop for DaemonFixture {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn actor() -> WorkActor {
    WorkActor {
        agent: "fixture".into(),
        session_id: "no-live-agent".into(),
        model_id: None,
        config_hash: "test".into(),
    }
}
fn verify_snapshot(store: &mut Store) {
    use nexus::delivery::{CheckState, DeliveryBody, DeliveryKind, DeliverySubject};
    let response = store
        .delivery_command(
            "project",
            &actor(),
            &DeliveryAction::Get {
                project_id: None,
                subject: DeliverySubject {
                    kind: DeliveryKind::PullRequest,
                    id: "pr".into(),
                },
            },
        )
        .unwrap();
    let DeliveryResponse::Detail {
        record: Some(record),
        ..
    } = response
    else {
        panic!("missing PR detail")
    };
    let DeliveryBody::PullRequest {
        snapshot: Some(snapshot),
        ..
    } = record.body
    else {
        panic!("missing provider snapshot")
    };
    assert_eq!(snapshot.head.commit, "b".repeat(40));
    assert_eq!(snapshot.base_ref.as_deref(), Some("development"));
    assert_eq!(snapshot.checks.len(), 1);
    assert_eq!(snapshot.checks[0].state, CheckState::Succeeded);
    assert!(store
        .list_sessions(nexus::coordination::domain::RecordScope::All, 120)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn daemon_resumes_a_persisted_watch_without_an_agent_and_shuts_down() {
    use axum::{routing::get, Json, Router};
    let app = Router::new()
        .route("/repos/example/repo/pulls/42",get(||async {Json(json!({"state":"open","merged":false,"updated_at":"2026-11-15T12:00:00Z","base":{"sha":"a".repeat(40),"ref":"development"},"head":{"sha":"b".repeat(40),"ref":"feature"}}))}))
        .route("/repos/example/repo/commits/{sha}/check-runs",get(||async {Json(json!({"total_count":1,"check_runs":[{"id":1,"name":"tests","head_sha":"b".repeat(40),"status":"completed","conclusion":"success","html_url":null}]}))}))
        .route("/repos/example/repo/commits/{sha}/status",get(||async {Json(json!({"total_count":0,"statuses":[]}))}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut fixture = DaemonFixture::new();
    fixture.seed();
    fixture.start(&api_base);
    fixture.await_refresh().await;
    fixture.stop().await;
    server.abort();
}
