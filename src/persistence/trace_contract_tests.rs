// @feature agent-traces
// @feature persistence
// @spec docs/features/agent-traces.md
// @spec docs/features/persistence.md
// @boundary trace-fixtures: JSON verifies the public query and batch contract.
use super::Store;
use crate::agent_traces::{
    TraceEventView, TraceInvocation, TraceOutcome, TracePage, TracePayloadState, TraceReadResponse,
};
use serde_json::json;

fn calls(response: TraceReadResponse) -> Vec<TraceInvocation> {
    let TraceReadResponse::Calls { calls, .. } = response else {
        panic!("expected a trace call page")
    };
    calls
}

fn events(response: TraceReadResponse) -> (Vec<TraceEventView>, TracePage) {
    let TraceReadResponse::Events { events, page } = response else {
        panic!("expected a trace event page")
    };
    (events, page)
}

#[test]
fn projects_a_single_call_when_its_finish_arrives_before_its_start() {
    let mut store = Store::open_memory().unwrap();
    let batch = |batch_id: &str, event| {
        serde_json::from_value(json!({
            "schema_version": 1, "batch_id": batch_id,
            "application_id": "harbor", "environment": "production", "events": [event]
        }))
        .unwrap()
    };

    let finish = || {
        batch(
            "batch-finish",
            json!({
                "event_id": "event-finish", "trace_id": "run-a", "agent_id": "clerk",
                "kind": "tool_finished", "occurred_at": "2026-11-26T20:00:01Z",
                "sequence": 2, "invocation_id": "call-a", "name": "lookup_order",
                "outcome": "failed", "duration_ms": 1000, "payload": {"code": "no_order"}
            }),
        )
    };
    let first = store.trace_append("project-a", finish()).unwrap();
    assert_eq!(store.trace_project_pending(16).unwrap(), 1);
    let discovered = store.list_projects().unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].project_id, "project-a");
    let replay = store.trace_append("project-a", finish()).unwrap();
    assert_eq!(first.received_at, replay.received_at);

    store
        .trace_append(
            "project-a",
            batch(
                "batch-start",
                json!({
                    "event_id": "event-start", "trace_id": "run-a", "agent_id": "clerk",
                    "kind": "tool_started", "occurred_at": "2026-11-26T20:00:00Z", "sequence": 1,
                    "invocation_id": "call-a", "name": "lookup_order", "version": "v2"
                }),
            ),
        )
        .unwrap();
    assert_eq!(store.trace_project_pending(16).unwrap(), 1);
    let query = || serde_json::from_value(json!({"action": "calls", "filter": {}})).unwrap();
    let rows = calls(store.trace_read("project-a", query()).unwrap());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, Some(TraceOutcome::Failed));
    assert_eq!(rows[0].started_event_id.as_deref(), Some("event-start"));
    assert_eq!(rows[0].finished_event_id.as_deref(), Some("event-finish"));
    assert_eq!(rows[0].version.as_deref(), Some("v2"));
    assert!(calls(store.trace_read("other-project", query()).unwrap()).is_empty());
}

#[test]
fn expires_payload_by_receipt_time_and_preserves_replay_and_portable_evidence() {
    let mut store = Store::open_memory().unwrap();
    let batch = || {
        serde_json::from_value(json!({
            "schema_version": 1, "batch_id": "batch-a",
            "application_id": "harbor", "environment": "prod",
            "events": [{
                "trace_id": "run-a", "agent_id": "clerk", "event_id": "event-a",
                "invocation_id": "call-a", "kind": "tool_finished", "name": "lookup",
                "occurred_at": "2020-01-01T00:00:00Z", "outcome": "succeeded",
                "payload": {"answer": "private"}
            }]
        }))
        .unwrap()
    };
    let read = || {
        serde_json::from_value(json!({
            "action": "get", "application_id": "harbor", "environment": "prod",
            "trace_id": "run-a", "include_payload": true
        }))
        .unwrap()
    };
    let receipt = store.trace_append("project-a", batch()).unwrap();
    assert_eq!(
        store
            .trace_expire_payloads("2021-01-01T00:00:00Z", 10)
            .unwrap(),
        0
    );
    let (captured, _) = events(store.trace_read("project-a", read()).unwrap());
    assert_eq!(
        captured[0].event.payload,
        Some(json!({"answer": "private"}))
    );
    assert_eq!(
        store
            .trace_expire_payloads("2100-01-01T00:00:00Z", 10)
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .trace_expire_payloads("2100-01-01T00:00:00Z", 10)
            .unwrap(),
        0
    );
    assert_eq!(store.trace_append("project-a", batch()).unwrap(), receipt);

    let snapshot = tempfile::NamedTempFile::new().unwrap();
    store.write_streamed_snapshot(snapshot.as_file()).unwrap();
    let mut restored = Store::open_memory().unwrap();
    restored.import_streamed_snapshot(snapshot.path()).unwrap();
    let (retained, _) = events(restored.trace_read("project-a", read()).unwrap());
    assert_eq!(retained[0].payload_state, TracePayloadState::Expired);
    assert!(retained[0].event.payload.is_none());
    assert_eq!(
        restored.trace_append("project-a", batch()).unwrap(),
        receipt
    );
    let query = serde_json::from_value(json!({"action": "calls"})).unwrap();
    let projected = calls(restored.trace_read("project-a", query).unwrap());
    assert_eq!(projected[0].outcome, Some(TraceOutcome::Succeeded));
}

#[test]
fn authorized_event_pages_follow_source_order_and_fit_transport() {
    let mut store = Store::open_memory().unwrap();
    let payload = "x".repeat(500_000);
    let event = |id: &str, sequence: i64, time: &str| {
        json!({
            "event_id": id, "trace_id": "run-a", "agent_id": "clerk",
            "kind": "message", "occurred_at": time, "sequence": sequence, "payload": payload,
        })
    };
    let batch = serde_json::from_value(json!({
        "schema_version": 1, "batch_id": "batch-a",
        "application_id": "harbor", "environment": "prod",
        "events": [event("second", 2, "2026-11-26T20:00:00Z"),
                   event("first", 1, "2026-11-26T20:00:01Z")]
    }))
    .unwrap();
    store.trace_append("project-a", batch).unwrap();

    let read = |include_payload: bool, cursor: Option<&str>| {
        serde_json::from_value(json!({
            "action": "get", "application_id": "harbor", "environment": "prod",
            "trace_id": "run-a", "limit": 10, "cursor": cursor, "include_payload": include_payload
        }))
        .unwrap()
    };
    let (metadata, _) = events(store.trace_read("project-a", read(false, None)).unwrap());
    assert_eq!(metadata.len(), 2);
    assert!(metadata[0].event.payload.is_none());

    let content = store.trace_read("project-a", read(true, None)).unwrap();
    let bytes = serde_json::to_vec(&content).unwrap();
    assert!(bytes.len() < 768 * 1024);
    assert_eq!(
        serde_json::from_slice::<TraceReadResponse>(&bytes).unwrap(),
        content
    );
    let (content, page) = events(content);
    assert_eq!(content.len(), 1);
    assert_eq!(content[0].event.event_id, "first");
    let (second, second_page) = events(
        store
            .trace_read("project-a", read(true, page.next_cursor.as_deref()))
            .unwrap(),
    );
    assert_eq!(second[0].event.event_id, "second");
    assert!(!second_page.truncated);
}

#[test]
fn aggregates_usage_for_one_app_agent_and_capability_revision() {
    let mut store = Store::open_memory().unwrap();
    let finish = |id: &str, version: Option<&str>, outcome: &str| {
        json!({
            "event_id": id, "trace_id": "run-a", "agent_id": "clerk",
            "kind": "tool_finished", "occurred_at": "2026-11-26T15:00:00-05:00",
            "invocation_id": id, "name": "lookup", "version": version, "outcome": outcome
        })
    };
    let batch = serde_json::from_value(json!({
        "schema_version": 1, "batch_id": "batch-a",
        "application_id": "harbor", "environment": "prod",
        "events": [finish("one", Some("1"), "succeeded"), finish("two", None, "timed_out")]
    }))
    .unwrap();
    store.trace_append("project-a", batch).unwrap();
    assert_eq!(store.trace_project_pending(5).unwrap(), 1);
    let result = |action: &str| {
        serde_json::from_value(json!({
        "action": action,
        "filter": {"application_id": "harbor", "agent_id": "clerk", "since": "2026-11-26T19:59:59Z"}
    })).unwrap()
    };
    let TraceReadResponse::List { traces, .. } =
        store.trace_read("project-a", result("list")).unwrap()
    else {
        panic!("expected a trace list page")
    };
    assert_eq!(traces[0].trace_id, "run-a");
    let TraceReadResponse::Usage { usage, .. } =
        store.trace_read("project-a", result("usage")).unwrap()
    else {
        panic!("expected a trace usage page")
    };
    assert_eq!(usage.len(), 2);

    let unversioned = serde_json::from_value(json!({
        "action": "usage", "filter": {"kind": "tool", "version": {"kind": "unversioned"}}
    }))
    .unwrap();
    let TraceReadResponse::Usage { usage, .. } =
        store.trace_read("project-a", unversioned).unwrap()
    else {
        panic!("expected a trace usage page")
    };
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].failed, 1);
    assert_eq!(usage[0].succeeded, 0);
}

#[test]
fn trace_detail_cursor_stays_contiguous_after_a_late_earlier_event() {
    let mut store = Store::open_memory().unwrap();
    let append =
        |store: &mut Store, batch_id: &str, observations: &[(&str, Option<i64>)]| {
            let events = observations.iter().map(|(id, sequence)| json!({
            "event_id": id, "trace_id": "run-a", "agent_id": "clerk",
            "kind": "message", "occurred_at": "2026-11-26T20:00:00Z", "sequence": sequence
        })).collect::<Vec<_>>();
            let batch = serde_json::from_value(json!({
                "schema_version": 1, "batch_id": batch_id,
                "application_id": "harbor", "environment": "prod", "events": events
            }))
            .unwrap();
            store.trace_append("project-a", batch).unwrap();
        };
    let read = |store: &mut Store, cursor: Option<&str>| {
        let query = serde_json::from_value(json!({
            "action": "get", "application_id": "harbor", "environment": "prod",
            "trace_id": "run-a", "limit": 1, "cursor": cursor
        }))
        .unwrap();
        events(store.trace_read("project-a", query).unwrap())
    };
    append(
        &mut store,
        "initial",
        &[
            ("second", Some(2)),
            ("third", Some(3)),
            ("fourth", None),
            ("fifth", None),
        ],
    );
    let (first, mut page) = read(&mut store, None);
    assert_eq!(first[0].event.event_id, "second");
    append(&mut store, "late", &[("first", Some(1))]);
    for expected in ["third", "fifth", "fourth"] {
        let (next, next_page) = read(&mut store, page.next_cursor.as_deref());
        assert_eq!(next[0].event.event_id, expected);
        page = next_page;
    }
    assert!(!page.truncated);
    assert_eq!(read(&mut store, None).0[0].event.event_id, "first");
}
