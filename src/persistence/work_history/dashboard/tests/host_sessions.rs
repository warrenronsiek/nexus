// @feature work-history
// @feature persistence
// @spec docs/features/work-history.md
// @spec docs/features/persistence.md
// @boundary fixture-json: closed work observations seed harness roots and their containers.
use super::*;

fn host_root(id: &str, host: &str, worktree: &str) -> WorkObservationInput {
    observation(
        &format!("{id}-open"),
        "session",
        id,
        json!({"event":"session_opened","title":format!("{host} session {id}-native"),
            "context":{"host_session_id":format!("{id}-native"),"worktree":worktree,
            "labels":{"nexus.accounting.role":"container","nexus.capture.adapter":"host-hooks-v1","nexus.capture.host":host}}}),
    )
}

fn activity(
    root: &str,
    id: &str,
    kind: &str,
    role: &str,
    seconds: Option<u64>,
) -> Vec<WorkObservationInput> {
    let mut events = vec![observation(
        &format!("{id}-declared"),
        "activity",
        id,
        json!({"event":"activity_declared","name":id,"kind":kind,"work_session_id":root,
            "context":{"labels":{"nexus.accounting.role":role}}}),
    )];
    if let Some(seconds) = seconds {
        events.push(observation(
            &format!("{id}-finished"),
            "activity",
            id,
            json!({"event":"finished","outcome":"unknown","elapsed":{"nanoseconds":(seconds * 1_000_000_000).to_string(),"method":"monotonic","clock_id":"clock"}}),
        ));
    }
    events
}

fn seed_harness_roots(store: &mut Store, project: &str) {
    let mut review = vec![host_root("review", "claude", "/repos/ui-review-11-2")];
    review.extend(activity(
        "review",
        "review-agent",
        "host.session",
        "container",
        Some(58),
    ));
    review.extend(activity(
        "review",
        "review-request",
        "request",
        "container",
        Some(50),
    ));
    review.extend(activity(
        "review",
        "review-turn",
        "host.turn",
        "container",
        Some(50),
    ));
    record(store, project, review);
    let mut work = vec![host_root("work", "claude", "/repos/comms-platform")];
    work.extend(activity(
        "work",
        "work-agent",
        "host.session",
        "container",
        Some(90),
    ));
    work.extend(activity(
        "work",
        "work-request-1",
        "request",
        "container",
        Some(30),
    ));
    work.extend(activity(
        "work",
        "work-request-2",
        "request",
        "container",
        Some(20),
    ));
    work.extend(activity("work", "work-tool", "tool", "execution", Some(10)));
    record(store, project, work);
    let mut named = vec![
        host_root("named", "codex", "/repos/emails"),
        observation(
            "named-title",
            "session",
            "named",
            json!({"event":"milestone","name":"Host session title updated","context":{"labels":{"nexus.session_title":"Inbox triage"}}}),
        ),
    ];
    named.extend(activity(
        "named",
        "named-request",
        "request",
        "container",
        None,
    ));
    named.extend(activity(
        "named",
        "named-tool",
        "tool",
        "execution",
        Some(4),
    ));
    record(store, project, named);
}

fn host_dashboard(store: &mut Store, project: &str) -> WorkDashboard {
    let response = dashboard(
        &mut store.connection,
        project,
        None,
        WorkSessionOrigin::Host,
    )
    .unwrap();
    let encoded = serde_json::to_value(&response).unwrap();
    let WorkResponse::Dashboard { dashboard } = serde_json::from_value(encoded).unwrap() else {
        panic!("expected dashboard");
    };
    *dashboard
}

fn row<'a>(view: &'a WorkDashboard, id: &str) -> &'a DashboardSession {
    view.sessions
        .iter()
        .find(|row| row.summary.subject.id() == id)
        .unwrap_or_else(|| panic!("missing {id}"))
}

fn kind<'a>(view: &'a WorkDashboard, kind: &str) -> &'a ActivityDurationStatistics {
    view.activity_kinds
        .iter()
        .find(|item| item.kind == kind)
        .unwrap_or_else(|| panic!("missing kind {kind}"))
}

#[test]
fn harness_roots_with_only_host_containers_are_hidden_from_sessions_and_counts() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    seed_harness_roots(&mut store, &project);
    let view = host_dashboard(&mut store, &project);
    let mut ids: Vec<_> = view
        .sessions
        .iter()
        .map(|row| row.summary.subject.id())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, ["named", "work"]);
    assert_eq!(view.hidden_session_count, 1);
    assert_eq!(view.session_duration.count, 2);
    assert_eq!(
        view.daily
            .iter()
            .map(|day| day.session_count)
            .sum::<usize>(),
        2
    );
    assert_eq!(kind(&view, "host.session").duration.count, 1);
    assert_eq!(kind(&view, "request").duration.count, 3);
    assert_eq!(view.activity_session_count, 2);
}

#[test]
fn generic_harness_titles_use_the_worktree_and_keep_the_host_session_identity() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    seed_harness_roots(&mut store, &project);
    let view = host_dashboard(&mut store, &project);
    let work = row(&view, "work");
    assert_eq!(work.summary.title, "claude · comms-platform");
    assert_eq!(
        work.summary.context.host_session_id.as_deref(),
        Some("work-native")
    );
    assert!(work.summary.started_at.is_some());
    assert_eq!(row(&view, "named").summary.title, "Inbox triage");
}

#[test]
fn session_rows_report_active_request_time_separately_from_host_lifetime() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    seed_harness_roots(&mut store, &project);
    let view = host_dashboard(&mut store, &project);
    let work = row(&view, "work");
    assert_eq!((work.request_count, work.timed_request_count), (2, 2));
    assert_eq!(work.active_seconds, Some(50.0));
    let named = row(&view, "named");
    assert_eq!((named.request_count, named.timed_request_count), (1, 0));
    assert_eq!(named.active_seconds, None);
    assert_eq!(view.active_duration.count, 2);
    assert_eq!(view.active_duration.known_count, 1);
    assert_eq!(view.active_duration.total_seconds, 50.0);
    let day = &view.daily[0];
    assert_eq!(
        (day.active_known_count, day.active_total_seconds),
        (1, 50.0)
    );
    assert_eq!(
        kind(&view, "request").accounting_role.as_deref(),
        Some("container")
    );
    assert_eq!(
        kind(&view, "tool").accounting_role.as_deref(),
        Some("execution")
    );
}
