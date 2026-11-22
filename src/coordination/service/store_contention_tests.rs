// @feature coordination
// @feature persistence
// @spec docs/features/coordination.md
// @spec docs/features/persistence.md
use crate::config::Config;
#[cfg(feature = "postgres")]
use crate::config::LoadedConfig;
use crate::coordination::api::ServiceRequest;
use serde_json::json;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

fn prompt() -> ServiceRequest {
    ServiceRequest::decode(
        "user_prompt",
        json!({
            "session_id":"contended-session",
            "agent":"codex",
            "project_root":"/compute/repo",
            "prompt":"Coordinate this work"
        }),
    )
    .unwrap()
}

#[test]
fn lifecycle_store_retries_after_contention_clears() {
    let service = super::tests::service_with(Config::default());
    let mut held = Some(service.store.lock().unwrap());
    let mut retries = 0;
    let store = service.store.single_for_test();
    let acquired = store
        .try_lock_with_retry(|| {
            retries += 1;
            assert_eq!(retries, 1, "the released store must be acquired on retry");
            drop(held.take());
            true
        })
        .unwrap();
    assert_eq!(retries, 1);
    drop(acquired);
    let response = service
        .handle_remote("contention-project", prompt())
        .to_json();
    assert_eq!(response["permitted"], true);
    assert_eq!(response["recorded"], true, "{response}");
}

#[test]
fn lifecycle_store_stops_when_retry_budget_is_exhausted() {
    let service = super::tests::service_with(Config::default());
    let _held = service.store.lock().unwrap();
    let mut retries = 0;
    let store = service.store.single_for_test();
    let error = store
        .try_lock_with_retry(|| {
            retries += 1;
            false
        })
        .err()
        .expect("contention must respect the retry budget");
    assert_eq!(retries, 1);
    assert!(matches!(error, std::sync::TryLockError::WouldBlock));
}

#[test]
fn lifecycle_contention_remains_bounded_and_fail_open() {
    let service = Arc::new(super::tests::service_with(Config::default()));
    let guard = service.store.lock().unwrap();
    let request_service = service.clone();
    let (sender, receiver) = mpsc::channel();
    let started = Instant::now();
    std::thread::spawn(move || {
        let response = request_service
            .handle_remote("contention-project", prompt())
            .to_json();
        sender.send(response).unwrap();
    });

    let response = receiver
        .recv_timeout(Duration::from_millis(500))
        .expect("contended lifecycle request must remain bounded");
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(response["permitted"], true);
    assert_eq!(response["recorded"], false, "{response}");
    assert_eq!(
        response["retryable"], true,
        "no store was acquired: {response}"
    );
    drop(guard);
}

#[test]
fn readiness_treats_active_store_work_as_busy_without_blocking() {
    let service = super::tests::service_with(Config::default());
    let guard = service.store.lock().unwrap();
    let started = Instant::now();

    let status = service.readiness_status().unwrap();

    assert_eq!(status, "busy");
    assert!(started.elapsed() < Duration::from_millis(50));
    drop(guard);
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_lifecycle_records_while_application_store_is_busy() {
    let database_url = std::env::var("NEXUS_TEST_DATABASE_URL")
        .expect("NEXUS_TEST_DATABASE_URL is required for the PostgreSQL contention test");
    let service = Arc::new(
        super::NexusService::connect(
            LoadedConfig {
                config: Config::default(),
                sources: Vec::new(),
                hash: "postgres-lifecycle-isolation-test".into(),
            },
            &database_url,
        )
        .unwrap(),
    );
    let guard = service.store.lock().unwrap();
    let request_service = service.clone();
    let (sender, receiver) = mpsc::channel();
    let request = std::thread::spawn(move || {
        sender
            .send(
                request_service
                    .handle_remote("lifecycle-isolation-project", prompt())
                    .to_json(),
            )
            .unwrap();
    });
    let response = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("lifecycle recording must not wait for application requests");
    drop(guard);
    request.join().unwrap();
    assert_eq!(response["recorded"], true, "{response}");
    let mut store = service.store.lock().unwrap();
    let events = store
        .list_events(Some("lifecycle-isolation-project"), 32)
        .unwrap();
    assert!(
        !events.is_empty(),
        "lifecycle evidence must be durable on the shared database"
    );
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_history_admission_reserves_interactive_and_lifecycle_capacity() {
    let service = super::NexusService::connect(
        LoadedConfig {
            config: Config::default(),
            sources: Vec::new(),
            hash: "postgres-history-admission".into(),
        },
        &std::env::var("NEXUS_TEST_DATABASE_URL").unwrap(),
    )
    .unwrap();
    let first = service.history_store().unwrap();
    let second = service.history_store().unwrap();
    let mut interactive = service.read_store().unwrap();
    interactive.list_projects().unwrap();
    let response = service
        .handle_remote("history-admission-test", prompt())
        .to_json();
    assert_eq!(response["recorded"], true, "{response}");
    drop((first, second, interactive));
    assert!(service.history_store().is_ok());
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_readiness_observes_lifecycle_store_failures() {
    let database_url = std::env::var("NEXUS_TEST_DATABASE_URL")
        .expect("NEXUS_TEST_DATABASE_URL is required for the PostgreSQL readiness test");
    let service = Arc::new(
        super::NexusService::connect(
            LoadedConfig {
                config: Config::default(),
                sources: Vec::new(),
                hash: "postgres-lifecycle-readiness-test".into(),
            },
            &database_url,
        )
        .unwrap(),
    );
    let guard = service.lifecycle_store().unwrap();
    assert_eq!(service.readiness_status().unwrap(), "ready");
    std::thread::sleep(Duration::from_millis(10));
    let error = service
        .readiness_status_with_store_stall_threshold(Duration::from_millis(1))
        .expect_err("a stalled lifecycle store must fail readiness");
    assert!(error.to_string().contains("stalled"), "{error:#}");
    drop(guard);
    assert_eq!(service.readiness_status().unwrap(), "ready");
    let poisoning_service = service.clone();
    assert!(std::thread::spawn(move || {
        let _guard = poisoning_service.lifecycle_store().unwrap();
        panic!("poison the lifecycle store for the readiness regression");
    })
    .join()
    .is_err());
    assert_eq!(service.readiness_status().unwrap(), "ready");
    assert!(service.lifecycle_store().is_ok());
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_readiness_always_uses_the_dedicated_probe() {
    let database_url = std::env::var("NEXUS_TEST_DATABASE_URL")
        .expect("NEXUS_TEST_DATABASE_URL is required for the PostgreSQL readiness test");
    let service = super::NexusService::connect(
        LoadedConfig {
            config: Config::default(),
            sources: Vec::new(),
            hash: "postgres-readiness-test".into(),
        },
        &database_url,
    )
    .unwrap();
    let application_guard = service.store.lock().unwrap();
    let started = Instant::now();

    let status = service.readiness_status().unwrap();

    assert_eq!(status, "ready");
    assert!(started.elapsed() < Duration::from_secs(1));
    drop(application_guard);

    let dedicated_guard = match &service.store {
        super::Database::Postgres(stores) => stores.probe.lock().unwrap(),
        super::Database::Single(_) => panic!("PostgreSQL must have a dedicated probe"),
    };
    let error = service
        .readiness_status()
        .expect_err("readiness must not fall back to the application connection");
    assert!(error.to_string().contains("probe unavailable"), "{error:#}");
    drop(dedicated_guard);
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_readiness_rejects_an_application_store_held_past_the_stall_threshold() {
    let database_url = std::env::var("NEXUS_TEST_DATABASE_URL")
        .expect("NEXUS_TEST_DATABASE_URL is required for the PostgreSQL readiness test");
    let service = super::NexusService::connect(
        LoadedConfig {
            config: Config::default(),
            sources: Vec::new(),
            hash: "postgres-store-stall-test".into(),
        },
        &database_url,
    )
    .unwrap();
    let guard = service.store.lock().unwrap();
    std::thread::sleep(Duration::from_millis(10));

    let error = service
        .readiness_status_with_store_stall_threshold(Duration::from_millis(1))
        .expect_err("a prolonged application-store hold must fail readiness");

    assert!(error.to_string().contains("stalled"), "{error:#}");
    drop(guard);
    assert_eq!(
        service
            .readiness_status_with_store_stall_threshold(Duration::from_millis(1))
            .unwrap(),
        "ready"
    );
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_pool_recovers_capacity_after_a_worker_panics() {
    let database_url = std::env::var("NEXUS_TEST_DATABASE_URL")
        .expect("NEXUS_TEST_DATABASE_URL is required for the PostgreSQL readiness test");
    let service = Arc::new(
        super::NexusService::connect(
            LoadedConfig {
                config: Config::default(),
                sources: Vec::new(),
                hash: "postgres-poisoned-store-test".into(),
            },
            &database_url,
        )
        .unwrap(),
    );
    let poisoning_service = service.clone();
    assert!(std::thread::spawn(move || {
        let _guard = poisoning_service.store.lock().unwrap();
        panic!("poison the application store for the readiness regression");
    })
    .join()
    .is_err());

    assert_eq!(service.readiness_status().unwrap(), "ready");
    assert!(service.store.lock().is_ok());
}

#[test]
fn poisoned_store_is_not_safe_to_retry() {
    let service = Arc::new(super::tests::service_with(Config::default()));
    let poisoning = service.clone();
    assert!(std::thread::spawn(move || {
        let _guard = poisoning.store.lock().unwrap();
        panic!("poison the store for the lifecycle regression");
    })
    .join()
    .is_err());
    let response = service
        .handle_remote("contention-project", prompt())
        .to_json();
    assert_eq!(response["recorded"], false);
    assert_ne!(response["retryable"], true, "{response}");
}

#[test]
fn script_contention_is_explicitly_safe_to_retry() {
    let service = super::tests::service_with(Config::default());
    let _held = service.store.lock().unwrap();
    let request = ServiceRequest::decode(
        "script_use",
        json!({
            "session_id":"busy-script", "agent":"codex", "project_root":"/repo",
            "invocation_id":"script-1", "script_name":"scripts/check.sh", "outcome":"succeeded"
        }),
    )
    .unwrap();
    let response = service
        .handle_remote("contention-project", request)
        .to_json();
    assert_eq!(response["recorded"], false);
    assert_eq!(response["retryable"], true, "{response}");
}

#[test]
fn reconciliation_releases_store_between_bounded_path_batches() {
    use crate::coordination::client_evidence::ClientFilesystemEvidence;
    use crate::coordination::domain::{Operation, PathIntent};
    let service = super::tests::service_with(Config::default());
    service.handle_remote("contention-project", prompt());
    let evidence = ClientFilesystemEvidence {
        session_id: "contended-session".into(),
        agent: "test".into(),
        changed_paths: (0..40)
            .map(|i| PathIntent {
                path: format!("src/{i}.rs"),
                operation: Operation::Write,
                line_start: None,
                line_end: None,
            })
            .collect(),
        changed_paths_truncated: false,
        derived_usage: None,
    };
    assert_eq!(
        service
            .reconcile_remote_evidence("contention-project", &evidence, false)
            .unwrap(),
        40
    );
    let mut store = service.store.lock().unwrap();
    let events = store.list_events(Some("contention-project"), 32).unwrap();
    let batches: Vec<_> = events
        .iter()
        .filter(|event| event.kind == "git_reconciled")
        .map(|event| {
            serde_json::to_value(&event.payload).unwrap()["changed_paths"]
                .as_array()
                .unwrap()
                .len()
        })
        .collect();
    assert_eq!(batches.iter().sum::<usize>(), 40);
    assert!(
        batches.iter().all(|size| *size <= 32),
        "reconciliation must yield between small store batches: {batches:?}"
    );
}

#[test]
#[cfg(feature = "sqlite")]
fn explicit_store_contention_releases_the_worker_within_a_bounded_wait() {
    let service = Arc::new(super::tests::service_with(Config::default()));
    let held = service.store.lock().unwrap();
    let reader = service.clone();
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let response = reader
            .handle_remote("contention-project", ServiceRequest::Status)
            .to_json();
        sender.send(response).unwrap();
    });
    let result = receiver.recv_timeout(Duration::from_secs(6));
    drop(held);
    worker.join().unwrap();
    let response =
        result.expect("an explicit request must not occupy a worker indefinitely behind the store");
    assert_eq!(
        response["ok"], false,
        "busy storage must be reported, not mistaken for a successful read"
    );
    assert_eq!(service.handle(ServiceRequest::Status).to_json()["ok"], true);
}

#[test]
#[cfg(feature = "sqlite")]
fn explicit_reads_survive_short_store_contention() {
    let service = Arc::new(super::tests::service_with(Config::default()));
    let held = service.store.lock().unwrap();
    let reader = service.clone();
    let (started, ready) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        started.send(()).unwrap();
        reader.handle(ServiceRequest::Status).to_json()
    });
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    // Browser primary and companion reads share this store; normal reads can
    // legitimately take longer than the lifecycle hook's short acquisition budget.
    std::thread::sleep(Duration::from_millis(750));
    drop(held);
    let response = worker.join().unwrap();
    assert_eq!(
        response["ok"], true,
        "ordinary page reads must survive short store contention: {response}"
    );
}
