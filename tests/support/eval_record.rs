// @feature evals
// @spec docs/features/evals.md
// @boundary typed-evaluation-fixture-input
// Integration crates use independent subsets of these shared fixture builders.
#![allow(dead_code)]
use nexus::evals::EvalRecordInput;
use serde_json::{json, Value};

pub fn record(id: &str, body: Value) -> EvalRecordInput {
    serde_json::from_value(
        json!({"id":id,"title":id,"occurred_at":"2026-10-22T12:00:00Z","body":body}),
    )
    .unwrap()
}

pub fn revision(id: &str, kind: &str, fields: Value) -> EvalRecordInput {
    let mut body = json!({"kind":kind,"identity":id,"change_id":"change"});
    body.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    record(id, body)
}
