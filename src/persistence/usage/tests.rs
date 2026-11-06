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
    assert!(!summary.truncated);
}

#[test]
fn usage_summary_has_a_server_owned_dimension_cap() {
    let mut store = Store::open_memory().unwrap();
    let start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    let end = start + chrono::Duration::days(7);

    for index in 0..65 {
        store
            .observe_capability_at(
                &observation(
                    "project-a",
                    &format!("invocation-{index:03}"),
                    &format!("tool-{index:03}"),
                ),
                CapabilityOutcome::Succeeded,
                start,
            )
            .unwrap();
    }

    let summary = store.usage_summary(Some("project-a"), start, end).unwrap();

    assert_eq!(summary.tools.len() + summary.skills.len(), 64);
    assert!(summary.truncated);
}

fn observe(store: &mut Store, kind: CapabilityKind, name: &str, agent: &str, session: &str) {
    let start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    let invocation = format!("{name}:{agent}:{session}:{}", Uuid::new_v4());
    let mut use_ = observation("project-a", &invocation, name);
    use_.kind = kind;
    use_.agent = agent.to_owned();
    use_.session_id = session.to_owned();
    use_.turn_id = Some(invocation);
    store
        .observe_capability_at(&use_, CapabilityOutcome::Observed, start)
        .unwrap();
}

#[test]
fn skill_sessions_survive_a_window_dominated_by_script_sessions() {
    let mut store = Store::open_memory().unwrap();
    // Distinct script sessions sort before skills and once exhausted a shared identity budget.
    for index in 0..4_100 {
        let session = format!("script-session-{index}");
        observe(
            &mut store,
            CapabilityKind::Script,
            "scripts/check.sh",
            "codex",
            &session,
        );
    }
    for (agent, session) in [("codex", "one"), ("codex", "one"), ("codex", "two")]
        .into_iter()
        .chain([("codex", "three"), ("codex", "three"), ("claude", "one")])
    {
        observe(
            &mut store,
            CapabilityKind::Skill,
            "test-audit",
            agent,
            session,
        );
    }
    let start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    let end = start + chrono::Duration::days(7);

    let summary = store.usage_summary(Some("project-a"), start, end).unwrap();

    let skill = &summary.skills[0];
    assert_eq!(
        (skill.count, skill.sessions),
        (6, 4),
        "agent-qualified sessions"
    );
    let script = &summary.tools[0];
    assert_eq!((script.count, script.sessions), (4_100, 4_100));
    assert!(!summary.truncated);
    let global = store.usage_summary(None, start, end).unwrap();
    assert_eq!(global.skills[0].sessions, 4);
}
