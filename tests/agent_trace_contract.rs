// @feature agent-traces
// @spec docs/features/agent-traces.md
// @boundary dynamic-json

use nexus::agent_traces::{TraceBatch, TraceQuery};
use serde_json::json;

#[test]
fn batch_accepts_out_of_order_observations_and_rejects_duplicate_event_ids() {
    let started = json!({
        "event_id": "start-1", "trace_id": "run-1", "agent_id": "support",
        "kind": "tool_started", "occurred_at": "2026-11-26T12:00:00Z",
        "sequence": 4, "invocation_id": "call-1", "name": "lookup"
    });
    let finished = json!({
        "event_id": "finish-1", "trace_id": "run-1", "agent_id": "support",
        "kind": "tool_finished", "occurred_at": "2026-11-26T12:00:01Z",
        "sequence": 5, "invocation_id": "call-1", "name": "lookup", "outcome": "failed"
    });
    let mut batch: TraceBatch = serde_json::from_value(json!({
        "schema_version": 1, "batch_id": "first", "application_id": "desk",
        "environment": "production", "events": [finished, started]
    }))
    .unwrap();
    batch.validate().unwrap();

    batch.events.push(batch.events[0].clone());
    assert!(batch
        .validate()
        .unwrap_err()
        .to_string()
        .contains("duplicate event_id"));
}

#[test]
fn content_reads_take_an_explicit_opt_in_and_unbounded_queries_fail() {
    let query: TraceQuery = serde_json::from_value(json!({
        "action": "get", "application_id": "desk", "environment": "production",
        "trace_id": "run-1"
    }))
    .unwrap();
    assert!(matches!(
        query,
        TraceQuery::Get {
            include_payload: false,
            ..
        }
    ));

    let query: TraceQuery = serde_json::from_value(json!({
        "action": "calls", "filter": { "limit": 10000 }
    }))
    .unwrap();
    assert!(query.validate().is_err());
}
