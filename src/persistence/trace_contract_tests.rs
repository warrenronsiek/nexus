// @feature agent-traces
// @feature persistence
// @spec docs/features/agent-traces.md
// @spec docs/features/persistence.md
// @boundary trace-fixtures: JSON verifies the public query and batch contract.
use super::Store;
use serde_json::json;

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
    let result = store.trace_read("project-a", query()).unwrap();
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["outcome"], "failed");
    assert_eq!(calls[0]["started_event_id"], "event-start");
    assert_eq!(calls[0]["finished_event_id"], "event-finish");
    assert_eq!(calls[0]["version"], "v2");
    assert!(store.trace_read("other-project", query()).unwrap()["calls"]
        .as_array()
        .unwrap()
        .is_empty());
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
    assert_eq!(
        store.trace_read("project-a", read()).unwrap()["events"][0]["payload"]["answer"],
        "private"
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
    let retained = restored.trace_read("project-a", read()).unwrap();
    assert_eq!(retained["events"][0]["payload_state"], "expired");
    assert!(retained["events"][0].get("payload").is_none());
    assert_eq!(
        restored.trace_append("project-a", batch()).unwrap(),
        receipt
    );
    let calls = serde_json::from_value(json!({"action": "calls"})).unwrap();
    let projected = restored.trace_read("project-a", calls).unwrap();
    assert_eq!(projected["calls"][0]["outcome"], "succeeded");
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
    let metadata = store.trace_read("project-a", read(false, None)).unwrap();
    assert_eq!(metadata["events"].as_array().unwrap().len(), 2);
    assert!(metadata["events"][0].get("payload").is_none());

    let content = store.trace_read("project-a", read(true, None)).unwrap();
    assert_eq!(content["events"].as_array().unwrap().len(), 1);
    assert_eq!(content["events"][0]["event_id"], "first");
    assert!(serde_json::to_vec(&content).unwrap().len() < 768 * 1024);
    let next = content["next_cursor"].as_str().unwrap();
    let second = store
        .trace_read("project-a", read(true, Some(next)))
        .unwrap();
    assert_eq!(second["events"][0]["event_id"], "second");
    assert_eq!(second["truncated"], false);
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
    let list = store.trace_read("project-a", result("list")).unwrap();
    assert_eq!(list["traces"][0]["trace_id"], "run-a");
    let usage = store.trace_read("project-a", result("usage")).unwrap();
    assert_eq!(usage["usage"].as_array().unwrap().len(), 2);

    let unversioned = serde_json::from_value(json!({
        "action": "usage", "filter": {"kind": "tool", "unversioned": true}
    }))
    .unwrap();
    let failures = store.trace_read("project-a", unversioned).unwrap();
    assert_eq!(failures["usage"].as_array().unwrap().len(), 1);
    assert_eq!(failures["usage"][0]["failed"], 1);
    assert_eq!(failures["usage"][0]["succeeded"], 0);
}
