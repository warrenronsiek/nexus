// @feature runtime
// @feature delivery-coordination
// @spec docs/features/runtime.md
// @spec docs/features/delivery-coordination.md
// @boundary generated-json-schema: derive the agent contract from the typed delivery domain.
use serde_json::{json, Value};

pub(super) fn definition() -> Value {
    json!({
        "name":"nexus_delivery",
        "description":"Register contributions and PR or rollout shepherds, discover current delivery responsibility, resolve a revision/path or feature to originating implementation agents, record promotions and deployment evidence, and exchange durable remediation requests. Origin results qualify evidence and ambiguity. Caller identity is bound by Nexus; recorded requests do not imply message delivery or an agent was resumed. Execution and deployment policy remain caller-owned.",
        "inputSchema":schemars::schema_for!(crate::delivery::DeliveryAction)
    })
}
