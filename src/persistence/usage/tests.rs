// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
use super::*;
use chrono::TimeZone;

fn observation(project_id: &str, invocation_id: &str, name: &str) -> CapabilityObservation {
    CapabilityObservation {
        project_id: project_id.to_owned(),
        agent: "codex".to_owned(),
        session_id: "session-a".to_owned(),
        turn_id: Some("turn-a".to_owned()),
        invocation_id: invocation_id.to_owned(),
        parent_invocation_id: None,
        kind: CapabilityKind::Tool,
        name: name.to_owned(),
        source: CapabilitySource::HostHook,
        evidence: CapabilityEvidence::NativeHook,
        config_hash: "config-a".to_owned(),
        model_id: None,
    }
}

#[test]
fn usage_window_includes_start_and_excludes_end() {
    let mut store = Store::open_memory().unwrap();
    let start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    let end = start + chrono::Duration::days(7);

    store
        .observe_capability_at(
            &observation("project-a", "at-start", "included"),
            CapabilityOutcome::Succeeded,
            start,
        )
        .unwrap();
    store
        .observe_capability_at(
            &observation("project-a", "at-end", "excluded-at-end"),
            CapabilityOutcome::Succeeded,
            end,
        )
        .unwrap();
    store
        .observe_capability_at(
            &observation("project-a", "before", "excluded-before"),
            CapabilityOutcome::Succeeded,
            start - chrono::Duration::nanoseconds(1),
        )
        .unwrap();
    store
        .observe_capability_at(
            &observation("project-b", "other-project", "excluded-project"),
            CapabilityOutcome::Succeeded,
            start,
        )
        .unwrap();

    let summary = store.usage_summary(Some("project-a"), start, end).unwrap();

    assert_eq!(summary.tools.len(), 1);
    assert_eq!(summary.tools[0].name, "included");
}
