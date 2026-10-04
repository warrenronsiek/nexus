// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
use chrono::{Duration, Utc};
use nexus::persistence::{
    CapabilityEvidence, CapabilityKind, CapabilityObservation, CapabilityOutcome, CapabilitySource,
    Store,
};

fn observation(invocation_id: &str) -> CapabilityObservation {
    CapabilityObservation {
        project_id: "project-a".to_owned(),
        agent: "codex".to_owned(),
        session_id: "session-a".to_owned(),
        turn_id: Some("turn-a".to_owned()),
        invocation_id: invocation_id.to_owned(),
        parent_invocation_id: None,
        kind: CapabilityKind::Tool,
        name: "shell".to_owned(),
        source: CapabilitySource::HostHook,
        evidence: CapabilityEvidence::NativeHook,
        config_hash: "config-a".to_owned(),
        model_id: Some("model-a".to_owned()),
    }
}

#[test]
fn repeated_lifecycle_delivery_counts_one_completed_capability_use() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let use_record = observation("tool-1");

    store.observe_capability_start(&use_record).unwrap();
    store.observe_capability_start(&use_record).unwrap();
    store
        .observe_capability_completion(&use_record, CapabilityOutcome::Succeeded)
        .unwrap();
    store
        .observe_capability_completion(&use_record, CapabilityOutcome::Succeeded)
        .unwrap();

    let now = Utc::now();
    let summary = store
        .usage_summary(None, now - Duration::days(7), now + Duration::minutes(1))
        .unwrap();

    assert_eq!(summary.tools.len(), 1);
    assert_eq!(summary.tools[0].kind, CapabilityKind::Tool);
    assert_eq!(summary.tools[0].name, "shell");
    assert_eq!(summary.tools[0].count, 1);
    assert_eq!(summary.tools[0].sessions, 1);
    assert_eq!(summary.tools[0].succeeded, 1);
    assert_eq!(summary.tools[0].failed, 0);
    assert!(summary.skills.is_empty());
}

#[test]
fn skills_deduplicate_per_turn_and_promote_stronger_evidence() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let mut inferred = observation("skill-read-1");
    inferred.kind = CapabilityKind::Skill;
    inferred.name = "tdd".to_owned();
    inferred.evidence = CapabilityEvidence::InstructionRead;
    let mut explicit = inferred.clone();
    explicit.invocation_id = "skill-explicit-2".to_owned();
    explicit.evidence = CapabilityEvidence::ExplicitInvocation;

    store
        .observe_capability_completion(&inferred, CapabilityOutcome::Observed)
        .unwrap();
    store
        .observe_capability_completion(&explicit, CapabilityOutcome::Observed)
        .unwrap();

    let now = Utc::now();
    let summary = store
        .usage_summary(
            Some("project-a"),
            now - Duration::days(7),
            now + Duration::minutes(1),
        )
        .unwrap();

    assert_eq!(summary.skills.len(), 1);
    assert_eq!(summary.skills[0].count, 1);
    assert_eq!(summary.skills[0].observed, 1);
    assert_eq!(
        summary.skills[0].evidence.get("explicit_invocation"),
        Some(&1)
    );
    assert!(!summary.skills[0].evidence.contains_key("instruction_read"));
}

#[test]
fn completion_without_start_is_recorded_and_project_scoping_is_exact() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temporary.path().join("nexus.db")).unwrap();
    let project_a = observation("same-tool-id");
    let mut project_b = project_a.clone();
    project_b.project_id = "project-b".to_owned();

    store
        .observe_capability_completion(&project_a, CapabilityOutcome::Failed)
        .unwrap();
    store
        .observe_capability_completion(&project_b, CapabilityOutcome::Succeeded)
        .unwrap();

    let now = Utc::now();
    let scoped = store
        .usage_summary(
            Some("project-a"),
            now - Duration::days(7),
            now + Duration::minutes(1),
        )
        .unwrap();
    let global = store
        .usage_summary(None, now - Duration::days(7), now + Duration::minutes(1))
        .unwrap();

    assert_eq!(scoped.tools[0].count, 1);
    assert_eq!(scoped.tools[0].sessions, 1);
    assert_eq!(scoped.tools[0].failed, 1);
    assert_eq!(global.tools[0].count, 2);
    assert_eq!(global.tools[0].sessions, 2);
    assert_eq!(global.tools[0].failed, 1);
    assert_eq!(global.tools[0].succeeded, 1);
}
