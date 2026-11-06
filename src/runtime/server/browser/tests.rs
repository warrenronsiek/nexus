// @feature runtime
// @feature observability-ui
// @spec docs/features/runtime.md
// @spec docs/features/observability-ui.md
// @boundary browser-read-fixture: JSON envelopes exercise the closed browser read decoder.
use super::*;
use std::os::unix::fs::PermissionsExt;
use std::time::Instant;

#[test]
fn credentials_store_only_a_salted_private_verifier() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("credentials.json");
    let password = b"an independent long password";
    configure_credentials(&path, "viewer", password).unwrap();
    let first = std::fs::read_to_string(&path).unwrap();
    assert!(!first.contains(std::str::from_utf8(password).unwrap()));
    assert!(first.contains("argon2id"));
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    configure_credentials(&path, "viewer", password).unwrap();
    assert_ne!(
        first,
        std::fs::read_to_string(&path).unwrap(),
        "password salts were reused"
    );
    assert!(credentials::Credentials::load(&path)
        .unwrap()
        .unwrap()
        .verifies("viewer", password));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(BrowserConfig::load(&path, None).is_err());
}

#[test]
fn credentials_reject_symlinks_and_password_file_overexposure() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("credentials.json");
    configure_credentials(&target, "viewer", b"an independent long password").unwrap();
    let link = temporary.path().join("link.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(configure_credentials(&link, "viewer", b"a replacement long password").is_err());
    assert!(BrowserConfig::load(&link, None).is_err());
    let password = temporary.path().join("password");
    std::fs::write(&password, b"an independent long password\r\n").unwrap();
    std::fs::set_permissions(&password, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        read_password_file(&password).unwrap(),
        b"an independent long password"
    );
    std::fs::set_permissions(&password, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_password_file(&password).is_err());
}

#[test]
fn sessions_expire_revoke_and_remain_bounded() {
    let auth = auth::Auth::new(None);
    let now = Instant::now();
    let token = auth.create_session(now).unwrap();
    assert!(auth.authenticated(Some(&token), now));
    auth.revoke(Some(&token));
    assert!(!auth.authenticated(Some(&token), now));
    let token = auth.create_session(now).unwrap();
    assert!(!auth.authenticated(
        Some(&token),
        now + Duration::from_secs(auth::SESSION_SECONDS)
    ));
    for _ in 0..256 {
        assert!(auth.create_session(now).is_some());
    }
    assert!(auth.create_session(now).is_none());
    assert!(auth
        .create_session(now + Duration::from_secs(auth::SESSION_SECONDS))
        .is_some());
}

#[test]
fn login_hash_admission_and_attempt_history_are_bounded() {
    let auth = auth::Auth::new(None);
    let now = Instant::now();
    let one = auth.admit_login(now).unwrap();
    let two = auth.admit_login(now).unwrap();
    assert!(auth.admit_login(now).is_none());
    drop((one, two));
    for _ in 0..10 {
        drop(auth.admit_login(now).unwrap());
    }
    assert!(auth.admit_login(now).is_none());
    assert!(auth.admit_login(now + Duration::from_secs(60)).is_some());
}

#[test]
fn production_origin_requires_https_and_has_no_forwarded_configuration() {
    for origin in [
        "http://nexus.example",
        "https://user:password@nexus.example",
        "https://nexus.example/path",
        "https://nexus.example?q=yes",
    ] {
        assert!(
            origin::BrowserOrigin::parse(Some(origin)).is_err(),
            "accepted {origin}"
        );
    }
    let temporary = tempfile::tempdir().unwrap();
    assert!(!BrowserConfig::load(&temporary.path().join("absent"), None)
        .unwrap()
        .configured());
}

#[test]
fn browser_credential_and_password_files_reject_owner_execute_permissions() {
    let temporary = tempfile::tempdir().unwrap();
    let credentials = temporary.path().join("credentials.json");
    configure_credentials(&credentials, "viewer", b"a private browser password").unwrap();
    std::fs::set_permissions(&credentials, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        BrowserConfig::load(&credentials, None).is_err(),
        "verifier permissions exceeded 0600"
    );
    let password = temporary.path().join("password");
    std::fs::write(&password, b"a private browser password").unwrap();
    std::fs::set_permissions(&password, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        read_password_file(&password).is_err(),
        "password permissions exceeded 0600"
    );
}

#[test]
fn session_history_reads_are_closed_project_scoped_browser_kinds() {
    let prepare = |envelope: serde_json::Value| {
        serde_json::from_value::<read::ReadEnvelope>(envelope)
            .map_err(anyhow::Error::from)
            .and_then(read::ReadEnvelope::prepare)
    };
    let (timeline, project) = prepare(serde_json::json!({
        "project_id": " project-a ",
        "query": {"kind": "coordination_session_timeline", "session_id": "session-1"}
    }))
    .unwrap();
    assert_eq!(project.as_deref(), Some("project-a"));
    assert!(matches!(
        timeline,
        ServiceRequest::SessionTimeline(query)
            if query.session_id == "session-1" && query.project_id.as_deref() == Some("project-a")
    ));
    let (index, _) = prepare(serde_json::json!({
        "project_id": "project-a",
        "query": {"kind": "coordination_session_index"}
    }))
    .unwrap();
    assert!(matches!(
        index,
        ServiceRequest::SessionIndex(query) if query.project_id.as_deref() == Some("project-a")
    ));

    for rejected in [
        serde_json::json!({"query": {"kind": "coordination_session_index"}}),
        serde_json::json!({"query": {"kind": "coordination_session_timeline", "session_id": "s"}}),
        serde_json::json!({"project_id": "p", "query": {"kind": "coordination_session_timeline"}}),
        serde_json::json!({"project_id": "p", "query": {"kind": "coordination_session_timeline", "session_id": ""}}),
        serde_json::json!({"project_id": "p", "query": {"kind": "coordination_session_index", "limit": 5}}),
    ] {
        assert!(prepare(rejected.clone()).is_err(), "{rejected}");
    }
}
