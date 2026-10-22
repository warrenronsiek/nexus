// @feature evals
// @spec docs/features/evals.md
// @boundary typed-evaluation-fixture-input
use nexus::evals::EvalRecordInput;
use serde_json::{json, Value};

pub fn record(id: &str, body: Value) -> EvalRecordInput {
    serde_json::from_value(
        json!({"id":id,"title":id,"occurred_at":"2026-10-22T12:00:00Z","body":body}),
    )
    .unwrap()
}
