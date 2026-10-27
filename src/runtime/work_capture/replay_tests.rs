// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary replay-fixture-http: a local typed service fixture verifies immutable callback replay without capturing workload payloads.
use super::*;
use crate::work_history::{WorkActor, WorkObservation, WorkOutcome};
use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;

async fn work_fixture(
    State(service): State<FixtureService>,
    Json(wire): Json<crate::runtime::protocol::WireRequest>,
) -> Json<Value> {
    tokio::time::sleep(service.delay).await;
    let ServiceRequest::Work(command) = ServiceRequest::decode(&wire.method, wire.params).unwrap()
    else {
        panic!("work request");
    };
    let result = service
        .store
        .lock()
        .unwrap()
        .work_command("project", &actor(), &command.command);
    Json(match result {
        Ok(response) => {
            let mut value = serde_json::to_value(response).unwrap();
            value["ok"] = json!(true);
            value
        }
        Err(_) => json!({"ok":false,"error_code":"conflict"}),
    })
}

fn actor() -> WorkActor {
    WorkActor {
        agent: "fixture".into(),
        session_id: "source-session".into(),
        model_id: None,
        config_hash: "test".into(),
    }
}

type StoreHandle = Arc<Mutex<crate::persistence::Store>>;

#[derive(Clone)]
struct FixtureService {
    store: StoreHandle,
    delay: Duration,
}

struct ReplayFixture {
    _temp: tempfile::TempDir,
    store: StoreHandle,
    client: NexusClient,
    path: PathBuf,
    context: HookContext,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for ReplayFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl ReplayFixture {
    async fn new() -> Self {
        Self::with_delay(Duration::ZERO).await
    }

    async fn with_delay(delay: Duration) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(Mutex::new(
            crate::persistence::Store::open(&temp.path().join("work.db")).unwrap(),
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/api/v1/request", post(work_fixture))
            .with_state(FixtureService {
                store: store.clone(),
                delay,
            });
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = replay_client(temp.path(), address);
        let context = HookContext {
            agent: "fixture".into(),
            session_id: "source-session".into(),
            project_root: None,
            turn_id: None,
            model: None,
        };
        let path = temp.path().join("outbox");
        Self {
            _temp: temp,
            store,
            client,
            path,
            context,
            server,
        }
    }

    fn command(&self, observation: WorkObservationInput) -> WorkCommand {
        WorkCommand {
            context: self.context.clone(),
            command: WorkAction::Record {
                observations: vec![observation],
            },
        }
    }

    fn record(&self, observation: WorkObservationInput) {
        self.store
            .lock()
            .unwrap()
            .work_command("project", &actor(), &self.command(observation).command)
            .unwrap();
    }

    fn evict_acknowledgment(&self, original: &WorkObservationInput) {
        let key = stable_id(&[&original.id]);
        let queue = Outbox::writer(&self.path).unwrap();
        queue
            .enqueue_once(
                &key,
                &serde_json::to_vec(&self.command(original.clone())).unwrap(),
            )
            .unwrap();
        queue
            .acknowledge(&queue.pending().unwrap().pop().unwrap())
            .unwrap();
        for index in 0..255 {
            std::fs::write(self.path.join(format!("other-{index}.json")), b"metadata").unwrap();
        }
        queue.enqueue("newer", b"metadata").unwrap();
        assert!(!self.path.join(format!("{key}.seen")).exists());
        queue
            .acknowledge(&queue.pending().unwrap().remove(0))
            .unwrap();
    }

    fn pending(&self, observation: WorkObservationInput) -> outbox::Pending {
        let key = stable_id(&[&observation.id]);
        let bytes = serde_json::to_vec(&self.command(observation)).unwrap();
        Outbox::writer(&self.path)
            .unwrap()
            .enqueue_once(&key, &bytes)
            .unwrap();
        outbox::Pending {
            path: self.path.join(format!("{key}.json")),
            bytes,
        }
    }

    async fn deliver(&self, observation: WorkObservationInput) {
        let entry = self.pending(observation);
        assert!(matches!(
            deliver(&self.client, &self.path, &entry, Duration::from_secs(2))
                .await
                .unwrap(),
            Delivery::Recorded
        ));
    }

    fn accepted(&self, id: String) -> WorkObservation {
        let response = self
            .store
            .lock()
            .unwrap()
            .work_command(
                "project",
                &actor(),
                &WorkAction::Observation {
                    project_id: None,
                    id,
                },
            )
            .unwrap();
        let crate::work_history::WorkResponse::Observation {
            observation: Some(accepted),
        } = response
        else {
            panic!("accepted observation");
        };
        *accepted
    }
}

fn replay_client(temp: &Path, address: std::net::SocketAddr) -> NexusClient {
    let secret = temp.join("secret");
    std::fs::write(&secret, b"fixture-credential-at-least-32-bytes").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut config = crate::Config::default();
    config.client = ClientConfig::Remote(crate::config::RemoteClientConfig {
        endpoint: format!("http://{address}"),
        project_id: "project".into(),
        secret_file: Some(secret),
    });
    NexusClient::without_analytics(
        Arc::new(LoadedConfig {
            config,
            hash: "test".into(),
            sources: Vec::new(),
        }),
        None,
    )
}

#[tokio::test]
async fn callback_after_ack_eviction_uses_the_accepted_boundary_and_allows_the_finish() {
    let fixture = ReplayFixture::new().await;
    let subject = crate::work_history::WorkSubject::Activity {
        id: "one-tool".into(),
    };
    let original = lifecycle::event(
        subject.clone(),
        "started",
        crate::work_history::WorkEvent::Started,
    );
    fixture.record(original.clone());
    fixture.evict_acknowledgment(&original);
    let mut replay = original.clone();
    replay.occurred_at += chrono::Duration::seconds(60);
    replay.producer.instance_id = "restarted-companion".into();
    fixture.deliver(replay).await;
    assert_eq!(fixture.accepted(original.id.clone()).input, original);
    let finish = lifecycle::event(
        subject,
        "finished",
        crate::work_history::WorkEvent::Finished {
            outcome: WorkOutcome::Succeeded,
            elapsed: None,
            exit_code: None,
            signal: None,
            assessment: None,
        },
    );
    fixture.deliver(finish.clone()).await;
    assert!(matches!(
        fixture.accepted(finish.id).input.body,
        crate::work_history::WorkEvent::Finished {
            outcome: WorkOutcome::Succeeded,
            ..
        }
    ));
}

#[tokio::test]
async fn replay_makes_progress_when_one_round_trip_fits_the_delivery_budget() {
    let fixture = ReplayFixture::with_delay(Duration::from_millis(150)).await;
    let original = lifecycle::event(
        crate::work_history::WorkSubject::Activity {
            id: "remote-tool".into(),
        },
        "started",
        crate::work_history::WorkEvent::Started,
    );
    fixture.record(original.clone());
    let mut replay = original.clone();
    replay.occurred_at += chrono::Duration::seconds(60);
    let mut entry = fixture.pending(replay);
    for _ in 0..3 {
        let delivery = deliver(
            &fixture.client,
            &fixture.path,
            &entry,
            Duration::from_millis(250),
        )
        .await
        .unwrap();
        if matches!(delivery, Delivery::Recorded) {
            assert_eq!(fixture.accepted(original.id.clone()).input, original);
            return;
        }
        entry = Outbox::writer(&fixture.path)
            .unwrap()
            .pending()
            .unwrap()
            .pop()
            .unwrap();
    }
    panic!("bounded retries must not repeat a conflict RPC forever before looking up its accepted observation");
}
