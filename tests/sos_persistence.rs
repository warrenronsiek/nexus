// @feature sos
// @feature persistence
// @spec docs/features/sos.md
// @spec docs/features/persistence.md
// @boundary dynamic-json
use nexus::persistence::Store;
use nexus::sos::{SosActor, SosMetadata, SosReportDraft};
use serde_json::json;

#[test]
fn sos_report_retains_normalized_source_fields_before_triage() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let metadata = SosMetadata::try_from(json!({
        "attempt": 3,
        "labels": ["network", "ci"],
        "nested": {"retryable": true}
    }))
    .unwrap();
    let actor = SosActor {
        agent: "codex".into(),
        session_id: "session-a".into(),
        model_id: Some("gpt-test".into()),
        config_hash: "config-a".into(),
    };
    let draft = SosReportDraft {
        message: " The release job cannot reach the registry. ".into(),
        kind: "deployment_failure".into(),
        environment: "ci/us-east-1".into(),
        version: "1.4.2".into(),
        model: "gpt-test".into(),
        trace: "connect timed out\nretry exhausted".into(),
        metadata: metadata.clone(),
    };

    let report = store.record_sos_report("project-a", draft, &actor).unwrap();

    assert_eq!(report.project_id, "project-a");
    assert_eq!(report.message, "The release job cannot reach the registry.");
    assert_eq!(report.kind, "deployment_failure");
    assert_eq!(report.environment, "ci/us-east-1");
    assert_eq!(report.version, "1.4.2");
    assert_eq!(report.model, "gpt-test");
    assert_eq!(report.trace, "connect timed out\nretry exhausted");
    assert_eq!(report.metadata, metadata);
    assert_eq!(report.reporter, actor);
    assert_eq!(report.issue_id, None);
}
