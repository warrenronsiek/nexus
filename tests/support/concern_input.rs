// @feature code-concerns
// @spec docs/features/code-concerns.md
// @boundary concern-input-fixtures: typed controlled wire fixtures for both backends.
use serde_json::{json, Value};
pub fn observation(id: &str, concern: &str, body: Value) -> Value {
    json!({"id":id,"concern_id":concern,"occurred_at":"2026-10-20T12:00:00Z","observed_at":"2026-10-20T12:00:01Z","context":{"host_session_id":"host","work_session_id":"work"},"body":body})
}
pub fn encounter(attempt: &str, area: &str) -> Value {
    json!({"symptom":"Wrong fallback","source":{"source":"test","repository":"repo","execution_attempt":attempt,"failure_id":"fallback-case"},"areas":[{"area":{"repository":"repo","kind":"subsystem","key":area,"display_name":area},"relation":"affected"}]})
}
pub fn report(id: &str, concern: &str, attempt: &str, area: &str) -> Value {
    observation(
        id,
        concern,
        json!({"event":"reported","title":"Fallback repeats","description":"Router ignores the configured fallback","encounter":encounter(attempt,area)}),
    )
}
