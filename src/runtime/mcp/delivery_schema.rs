// @feature runtime
// @feature delivery-coordination
// @spec docs/features/runtime.md
// @spec docs/features/delivery-coordination.md
// @boundary generated-json-schema: derive the agent contract from the typed delivery domain.
use serde_json::{json, Value};

pub(super) fn definition() -> Value {
    json!({
        "name":"nexus_delivery",
        "description":"Register contributions and PR or rollout shepherds, discover current delivery responsibility, resolve a revision, PR, branch, or feature to implementation agents, and queue and acknowledge addressed remediation requests. Read remediation_status for receipt and recent recipient activity. Environment context assesses dependencies against the candidate revision and lists active advisories. Check history counts distinct PRs with retained failures. Execution and deployment policy remain caller-owned.",
        "inputSchema":schemars::schema_for!(crate::delivery::DeliveryAction)
    })
}
