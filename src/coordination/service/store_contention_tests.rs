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
fn lifecycle_waits_briefly_for_store_contention_then_records() {
    let service = Arc::new(super::tests::service_with(Config::default()));
    let guard = service.store.lock().unwrap();
    let request_service = service.clone();
    let worker = std::thread::spawn(move || {
        request_service
            .handle_remote("contention-project", prompt())
            .to_json()
    });

    std::thread::sleep(Duration::from_millis(20));
    drop(guard);
    let response = worker.join().unwrap();
    assert_eq!(response["permitted"], true);
    assert_eq!(response["recorded"], true, "{response}");
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

    let dedicated_guard = match &service.readiness {
        super::ServiceReadiness::Dedicated(probe) => probe.lock().unwrap(),
        super::ServiceReadiness::SharedStore => panic!("PostgreSQL must have a dedicated probe"),
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
fn postgres_readiness_rejects_a_poisoned_application_store() {
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

    let error = service
        .readiness_status()
        .expect_err("a poisoned application store must fail readiness");

    assert!(
        error.to_string().contains("store lock poisoned"),
        "{error:#}"
    );
}
