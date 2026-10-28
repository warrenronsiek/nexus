// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @boundary isolated-daemon-update-control: only temporary test services are inspected or restarted
mod support;
use nexus::{runtime::daemon_control, Config};
use std::time::Duration;
use support::Harness;

#[tokio::test]
async fn exact_instance_restart_preserves_the_running_daemon_configuration() {
    let mut harness = Harness::start();
    let path = harness._temp.path().join("config.toml");
    let loaded = Config::load(Some(&path), Some(&harness.root)).unwrap();
    let snapshot = daemon_control::inspect(&loaded).await.unwrap().unwrap();
    assert_eq!(snapshot.config.storage.database_path, harness.database);
    assert_eq!(snapshot.directory, harness.root.canonicalize().unwrap());
    assert_eq!(snapshot.config_hash, loaded.hash);
    let mut wrong = snapshot.clone();
    wrong.instance_id = "a-different-instance".into();
    assert!(daemon_control::stop(&loaded, &wrong).await.is_err());
    assert_eq!(harness.status()["ok"], true);
    daemon_control::stop(&loaded, &snapshot).await.unwrap();
    let status = harness.daemon.wait().unwrap();
    assert!(status.success(), "controlled shutdown failed: {status}");
    assert!(!snapshot.config.runtime.socket_path.exists());
    daemon_control::restart(&snapshot, std::path::Path::new(env!("CARGO_BIN_EXE_nexus")))
        .await
        .unwrap();
    let restarted = daemon_control::inspect(&loaded).await.unwrap().unwrap();
    assert_ne!(restarted.instance_id, snapshot.instance_id);
    assert_eq!(restarted.config_hash, snapshot.config_hash);
    assert_eq!(restarted.config.storage.database_path, harness.database);
    daemon_control::stop(&loaded, &restarted).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
}
