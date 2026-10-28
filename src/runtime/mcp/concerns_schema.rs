// @feature code-concerns
// @feature runtime
// @spec docs/features/code-concerns.md
// @spec docs/features/runtime.md
// @boundary generated-json-schema: advertise the same closed action types decoded by the service
use serde_json::{json, Value};

pub(super) fn definition() -> Value {
    json!({
        "name":"nexus_concerns",
        "description":"Record and investigate code defects and recurring problem areas across work sessions. Distinct from development-environment Papercuts and live-application SOS reports. Record immutable encounters, locations, remedies and why they were tried; retrieve concern histories, area recurrence summaries, and timelines. Shared source failures are counted separately from reporter observations. Callers own fixes and review decisions. Nexus binds submitter/project provenance.",
        "inputSchema":super::concern_reporting::schema()
    })
}
