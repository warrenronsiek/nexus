// @feature installation
// @spec docs/features/installation.md
// @boundary deployment-fixtures: fake service executables and loopback release assets never touch the live deployment.
use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn mismatched_endpoint_is_rejected_before_downloading_or_applying() {
    let directory = tempfile::tempdir().unwrap();
    let command = directory.path().join("service updater");
    std::fs::write(&command, format!("#!/bin/sh\nprintf '%s\\n' '{{\"format_version\":1,\"endpoint\":\"https://other.example\",\"target\":\"aarch64-unknown-linux-gnu\",\"image_id\":\"sha256:{}\"}}'\n", "e".repeat(64))).unwrap();
    std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).unwrap();
    let fixture = super::super::release::tests::fixture();
    let release = super::super::release::ReleaseManifest::decode(
        &serde_json::to_vec(&fixture).unwrap(),
        &"a".repeat(40),
    )
    .unwrap();
    let error = Deployment::prepare(
        &command,
        "https://service.example",
        &super::super::registry::Registry::local("http://127.0.0.1:1"),
        &release,
    )
    .await
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("exact configured endpoint"));
}
