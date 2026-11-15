// @feature delivery-coordination
// @feature runtime
// @spec docs/features/delivery-coordination.md
// @spec docs/features/runtime.md
// @boundary provider-fixture-json: local HTTP fixtures exercise externally shaped GitHub snapshots.
use nexus::runtime::delivery_watch::{GithubSource, GithubWatcher};
use serde_json::json;

#[tokio::test]
async fn github_poll_pins_checks_to_observed_head_and_rejects_a_changed_head() {
    use axum::{extract::State, routing::get, Json, Router};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let count = Arc::new(AtomicUsize::new(0));
    async fn pull(State(count): State<Arc<AtomicUsize>>) -> Json<serde_json::Value> {
        let n = count.fetch_add(1, Ordering::SeqCst);
        Json(
            json!({"state":"open","merged":false,"updated_at":"2026-11-15T12:00:00Z","base":{"sha":"a".repeat(40)},"head":{"sha":if n == 0 {"b".repeat(40)} else {"c".repeat(40)}}}),
        )
    }
    let app = Router::new()
        .route("/repos/example/repo/pulls/42", get(pull))
        .route("/repos/example/repo/commits/{sha}/check-runs", get(|| async { Json(json!({"total_count":1,"check_runs":[{"id":123,"name":"tests","head_sha":"b".repeat(40),"status":"completed","conclusion":"success","html_url":"https://example.test/check/123"}]})) }))
        .route("/repos/example/repo/commits/{sha}/status", get(|| async { Json(json!({"total_count":0,"statuses":[]})) }))
        .with_state(count);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let watcher = GithubWatcher::new(&url, None).unwrap();
    let result = watcher
        .poll(
            &GithubSource {
                owner: "example".into(),
                repository: "repo".into(),
                number: 42,
            },
            "repo-id",
        )
        .await;
    assert!(result.unwrap_err().to_string().contains("head changed"));
    server.abort();
}

#[test]
fn git_capture_retains_selected_revision_and_hunks_without_claiming_an_author() {
    use nexus::runtime::delivery_capture::capture;
    use std::process::Command;
    let directory = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@example.test"]);
    std::fs::write(
        directory.path().join("example.rs"),
        "// @feature delivery-coordination\nfn value() -> u32 { 1 }\n",
    )
    .unwrap();
    git(&["add", "example.rs"]);
    git(&["commit", "-qm", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    std::fs::write(
        directory.path().join("example.rs"),
        "// @feature delivery-coordination\nfn value() -> u32 { 2 }\n",
    )
    .unwrap();
    git(&["commit", "-qam", "change"]);
    let captured = capture(directory.path(), "repo-id", &base, "HEAD").unwrap();
    assert_eq!(captured.base.commit, base);
    assert_eq!(captured.head.commit, git(&["rev-parse", "HEAD"]));
    assert_eq!(captured.files.len(), 1);
    assert_eq!(captured.files[0].path, "example.rs");
    assert_eq!(captured.files[0].hunks[0].new_start, 2);
    assert_eq!(captured.files[0].features, ["delivery-coordination"]);
    let encoded = serde_json::to_string(&captured).unwrap();
    assert!(!encoded.contains("fn value"));
    assert!(!encoded.contains("fixture@example"));
}

#[test]
fn watch_rejects_a_source_for_a_different_registered_pull_request() {
    use nexus::{delivery::DeliveryAction, persistence::Store, work_history::WorkActor};
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(&directory.path().join("watch.db")).unwrap();
    let actor = WorkActor {
        agent: "codex".into(),
        session_id: "owner".into(),
        model_id: None,
        config_hash: "test".into(),
    };
    let revision = |sha: char| json!({"repository":"repo-id","object_format":"sha1","commit":sha.to_string().repeat(40)});
    let action: DeliveryAction = serde_json::from_value(json!({"action":"record","observations":[{"id":"pr","occurred_at":"2026-11-15T12:00:00Z","body":{"event":"pull_request_registered","pull_request":{"id":"pr-id","repository":"repo-id","provider":"github","number":42,"url":"https://github.com/example/repo/pull/42","title":"Release","base":revision('a'),"head":revision('b'),"contribution_ids":[]}}}]})).unwrap();
    store.delivery_command("project", &actor, &action).unwrap();
    let action: DeliveryAction = serde_json::from_value(json!({"action":"record","observations":[{"id":"watch","occurred_at":"2026-11-15T12:00:00Z","body":{"event":"watch_registered","watch":{"id":"watch-id","pull_request_id":"pr-id","repository_id":"repo-id","source":{"owner":"example","repository":"repo","number":43},"interval_seconds":30}}}]})).unwrap();
    assert!(store.delivery_command("project", &actor, &action).is_err());
}

struct WatchFixture {
    directory: tempfile::TempDir,
    store: nexus::persistence::Store,
    actor: nexus::work_history::WorkActor,
}
impl WatchFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store = nexus::persistence::Store::open(&directory.path().join("watch.db")).unwrap();
        let actor = nexus::work_history::WorkActor {
            agent: "codex".into(),
            session_id: "owner".into(),
            model_id: None,
            config_hash: "test".into(),
        };
        Self {
            directory,
            store,
            actor,
        }
    }
    fn command(&mut self, action: serde_json::Value) -> serde_json::Value {
        let action = serde_json::from_value(action).unwrap();
        serde_json::to_value(
            self.store
                .delivery_command("project", &self.actor, &action)
                .unwrap(),
        )
        .unwrap()
    }
    fn record(&mut self, id: &str, body: serde_json::Value) {
        self.command(json!({"action":"record","observations":[{"id":id,"occurred_at":"2026-11-15T12:00:00Z","body":body}]}));
    }
    fn register(&mut self) {
        self.record("pr",json!({"event":"pull_request_registered","pull_request":{"id":"pr-id","repository":"repo-id","provider":"github","number":42,"url":"https://github.com/example/repo/pull/42","title":"Release","base":watch_revision('a'),"head":watch_revision('b'),"contribution_ids":[]}}));
        self.record("watch",json!({"event":"watch_registered","watch":{"id":"watch-id","pull_request_id":"pr-id","repository_id":"repo-id","source":{"owner":"example","repository":"repo","number":42},"interval_seconds":30}}));
    }
    fn poll_result(&self, head: char) -> nexus::delivery::DeliveryWatchOutcome {
        nexus::delivery::DeliveryWatchOutcome::Observed { snapshot:serde_json::from_value(json!({"observed_at":chrono::Utc::now(),"state":"open","base":watch_revision('a'),"head":watch_revision(head),"checks":[]})).unwrap(),provider_updated_at:chrono::Utc::now(),digest:head.to_string().repeat(64) }
    }
    fn assert_pr_event_count(&mut self, expected: usize) {
        let result =
            self.command(json!({"action":"events","subject":{"kind":"pull_request","id":"pr-id"}}));
        assert_eq!(result["observations"].as_array().unwrap().len(), expected);
    }
}
fn watch_revision(sha: char) -> serde_json::Value {
    json!({"repository":"repo-id","object_format":"sha1","commit":sha.to_string().repeat(40)})
}

#[test]
fn contribution_capture_treats_bracketed_paths_as_literal_filenames() {
    let directory = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@example.test"]);
    std::fs::create_dir(directory.path().join("route")).unwrap();
    for file in ["route/[id].txt", "route/i.txt"] {
        std::fs::write(directory.path().join(file), "a\nb\nc\n").unwrap();
    }
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    std::fs::write(directory.path().join("route/[id].txt"), "A\nb\nc\n").unwrap();
    std::fs::write(directory.path().join("route/i.txt"), "a\nb\nC\n").unwrap();
    git(&["commit", "-qam", "changes"]);
    let captured =
        nexus::runtime::delivery_capture::capture(directory.path(), "repo", &base, "HEAD").unwrap();
    let bracketed = captured
        .files
        .iter()
        .find(|file| file.path == "route/[id].txt")
        .unwrap();
    assert_eq!(
        bracketed.hunks.len(),
        1,
        "another file's hunks must not be attributed to a bracketed filename"
    );
    assert_eq!(bracketed.hunks[0].new_start, 1);
}

#[test]
fn persisted_watch_recovers_after_reopen_and_deduplicates_unchanged_state() {
    use chrono::{Duration, Utc};
    let mut fixture = WatchFixture::new();
    fixture.register();
    let first = fixture
        .store
        .claim_delivery_watches(Utc::now() + Duration::seconds(1))
        .unwrap()
        .remove(0);
    fixture
        .store
        .apply_delivery_watch_poll(&first, fixture.poll_result('b'))
        .unwrap();
    fixture.assert_pr_event_count(2);
    let path = fixture.directory.path().join("watch.db");
    drop(fixture.store);
    fixture.store = nexus::persistence::Store::open(&path).unwrap();
    let second = fixture
        .store
        .claim_delivery_watches(Utc::now() + Duration::seconds(31))
        .unwrap()
        .remove(0);
    assert!(second.last_success_at.is_some());
    assert_eq!(second.last_digest, Some("b".repeat(64)));
    fixture
        .store
        .apply_delivery_watch_poll(&second, fixture.poll_result('b'))
        .unwrap();
    fixture.assert_pr_event_count(2);
}

#[test]
fn failed_refresh_preserves_last_success_and_a_stopped_poll_cannot_publish() {
    use chrono::{Duration, Utc};
    let mut fixture = WatchFixture::new();
    fixture.register();
    let first = fixture
        .store
        .claim_delivery_watches(Utc::now() + Duration::seconds(1))
        .unwrap()
        .remove(0);
    fixture
        .store
        .apply_delivery_watch_poll(&first, fixture.poll_result('b'))
        .unwrap();
    let third = fixture
        .store
        .claim_delivery_watches(Utc::now() + Duration::seconds(62))
        .unwrap()
        .remove(0);
    fixture
        .store
        .apply_delivery_watch_poll(
            &third,
            nexus::delivery::DeliveryWatchOutcome::Failed {
                reason: "GitHub HTTP 429".into(),
            },
        )
        .unwrap();
    let health = fixture.command(json!({"action":"watches"}));
    assert_eq!(health["watches"][0]["last_error"], "GitHub HTTP 429");
    assert!(health["watches"][0]["last_success_at"].is_string());
    let fourth = fixture
        .store
        .claim_delivery_watches(Utc::now() + Duration::seconds(93))
        .unwrap()
        .remove(0);
    fixture.record(
        "stop",
        json!({"event":"watch_stopped","watch_id":"watch-id"}),
    );
    fixture
        .store
        .apply_delivery_watch_poll(&fourth, fixture.poll_result('c'))
        .unwrap();
    fixture.assert_pr_event_count(2);
    assert!(fixture
        .store
        .claim_delivery_watches(Utc::now() + Duration::days(1))
        .unwrap()
        .is_empty());
}

#[test]
fn watch_listing_discloses_the_server_bound() {
    let mut fixture = WatchFixture::new();
    fixture.register();
    for n in 0..128 {
        fixture.record(&format!("extra-{n}"),json!({"event":"watch_registered","watch":{"id":format!("extra-{n}"),"pull_request_id":"pr-id","repository_id":"repo-id","source":{"owner":"example","repository":"repo","number":42},"interval_seconds":30}}));
    }
    let response = fixture.command(json!({"action":"watches"}));
    assert_eq!(response["watches"].as_array().unwrap().len(), 128);
    assert_eq!(response["truncated"], true);
}
