// @feature evals
// @spec docs/features/evals.md
// @feature runtime
// @feature usage-analytics
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/runtime.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
// @boundary dynamic-json-schema
use serde_json::{json, Value};

pub(super) fn tool_definitions() -> Value {
    let mut definitions = lifecycle_tool_definitions();
    definitions.extend(coordination_tool_definitions());
    definitions.push(super::evals_schema::definition());
    definitions.push(super::work_schema::definition());
    Value::Array(definitions)
}

fn lifecycle_tool_definitions() -> Vec<Value> {
    let tool_hook_schema = json!({
        "type":"object",
        "required":["session_id","tool_use_id","tool_name"],
        "properties":{
            "session_id":{"type":"string"},"project_root":{"type":"string"},"agent":{"type":"string"},
            "turn_id":{"type":"string"},"model":{"type":"string"},
            "tool_use_id":{"type":"string"},"parent_tool_use_id":{"type":"string"},"tool_name":{"type":"string"},"tool_input":{},"tool_output":{},"error":{"type":"string"}
        },
        "additionalProperties":true
    });
    json_array(json!([
        {"name":"nexus_user_prompt","description":"Record a privacy-limited task synopsis for agent coordination.","inputSchema":{"type":"object","required":["session_id","prompt"],"properties":{"session_id":{"type":"string"},"project_root":{"type":"string"},"agent":{"type":"string"},"prompt":{"type":"string"}}}},
        {"name":"nexus_pre_tool_use","description":"Inspect an upcoming tool call and return non-blocking coordination advisories. Always permits execution.","inputSchema":tool_hook_schema},
        {"name":"nexus_post_tool_use","description":"Record successful tool completion. Never controls execution.","inputSchema":tool_hook_schema},
        {"name":"nexus_post_tool_failure","description":"Record failed tool completion. Never controls execution.","inputSchema":tool_hook_schema},
        {"name":"nexus_skill_use","description":"Record an observed skill activation with typed evidence. Never controls execution.","inputSchema":{"type":"object","required":["session_id","invocation_id","skill_name","evidence"],"properties":{"session_id":{"type":"string"},"project_root":{"type":"string"},"agent":{"type":"string"},"turn_id":{"type":"string"},"model":{"type":"string"},"invocation_id":{"type":"string"},"skill_name":{"type":"string"},"evidence":{"type":"string","enum":["native_hook","explicit_invocation","instruction_read","asset_execution"]},"actor":{"type":"string"}}}},
        {"name":"nexus_session_stop","description":"Mark a session stopped and release its advisory claims.","inputSchema":{"type":"object","required":["session_id"],"properties":{"session_id":{"type":"string"},"project_root":{"type":"string"},"agent":{"type":"string"}}}},
    ]))
}

fn coordination_tool_definitions() -> Vec<Value> {
    // Agent discovery includes read-only coordination and observability plus the simple
    // caller memory tools. Memory administration stays reserved for local user surfaces.
    json_array(json!([
        {"name":"nexus_status","description":"Return daemon and projection counts.","inputSchema":{"type":"object"}},
        {"name":"nexus_sessions","description":"List agent sessions.","inputSchema":{"type":"object","properties":{"project_id":{"type":"string","maxLength":256},"scope":{"type":"string","enum":["active","all"],"default":"active"}},"additionalProperties":false}},
        {"name":"nexus_claims","description":"List advisory path claims.","inputSchema":{"type":"object","properties":{"project_id":{"type":"string","maxLength":256},"scope":{"type":"string","enum":["active","all"],"default":"active"}},"additionalProperties":false}},
        {"name":"nexus_conflicts","description":"List detected overlaps.","inputSchema":{"type":"object","properties":{"project_id":{"type":"string","maxLength":256},"scope":{"type":"string","enum":["open","all"],"default":"open"}},"additionalProperties":false}},
        {"name":"nexus_resolve","description":"Record an optional resolution chosen by an agent or user.","inputSchema":{"type":"object","required":["conflict_id"],"properties":{"conflict_id":{"type":"string"},"resolution":{"type":"string"}},"additionalProperties":false}},
        {"name":"nexus_release","description":"Release a session's advisory claims.","inputSchema":{"type":"object","required":["session_id"],"properties":{"session_id":{"type":"string"},"path":{"type":"string"}},"additionalProperties":false}},
        {"name":"nexus_analyze","description":"Explicitly ask the configured read-only Codex or Claude analyst to summarize one conflict and suggest optional resolutions.","inputSchema":{"type":"object","required":["conflict_id"],"properties":{"conflict_id":{"type":"string"}},"additionalProperties":false}},
        {"name":"nexus_events","description":"Return recent coordination events.","inputSchema":{"type":"object","properties":{"project_id":{"type":"string","maxLength":256},"limit":{"type":"integer","minimum":1,"maximum":64}},"additionalProperties":false}},
        {"name":"nexus_projects","description":"List repositories with observed Nexus activity.","inputSchema":{"type":"object","additionalProperties":false}},
        {"name":"nexus_dashboard","description":"Return bounded coordination state for the terminal interfaces.","inputSchema":{"type":"object","properties":{"project_id":{"type":"string","maxLength":256}},"additionalProperties":false}},
        {"name":"nexus_usage","description":"Return observed tool, script, and skill usage for the rolling seven-day window.","inputSchema":{"type":"object","properties":{"project_id":{"type":"string","maxLength":256}},"additionalProperties":false}},
        {"name":"nexus_memory_add","description":"Record one explicit durable memory. Nexus handles background consolidation.","inputSchema":{"type":"object","required":["scope","content"],"properties":{"scope":{"type":"string","enum":["global","project"]},"content":{"type":"string","maxLength":512}},"additionalProperties":false}},
        {"name":"nexus_memory_search","description":"Search authoritative raw memories with a case-insensitive Rust regular expression.","inputSchema":{"type":"object","required":["regex"],"properties":{"scope":{"type":"string","enum":["layered","global","project"],"default":"layered"},"regex":{"type":"string","maxLength":crate::coordination::api::MAX_MEMORY_SEARCH_REGEX_BYTES},"limit":{"type":"integer","minimum":1,"maximum":crate::memory::MAX_MEMORY_SEARCH_RESULTS,"default":50}},"additionalProperties":false}},
        {"name":"nexus_memory_expand","description":"Expand one derived memory summary into its two source children.","inputSchema":{"type":"object","required":["summary_id"],"properties":{"summary_id":{"type":"string"}},"additionalProperties":false}},
        super::analytics_schema::definition(),
        papercut_tool_definition(),
        sos_tool_definition(),
        {"name":"nexus_config","description":"Return resolved non-secret Nexus configuration and its hash.","inputSchema":{"type":"object"}}
    ]))
}

fn papercut_tool_definition() -> Value {
    json!({
        "name":"nexus_papercuts",
        "description":"List, dump, report, upvote, remove, or respond to project development-environment papercuts. List accepts an optional after cursor for the next older page. Nexus supplies caller and project context. Duplicate review is requested automatically and dispatched asynchronously on a best-effort basis after a new report is durable; only the original reporter can approve consolidation. dump is complete and unbounded.",
        "inputSchema":{
            "type":"object",
            "oneOf":[
                papercut_action_schema("list", &[], json!({
                    "project_id":{"type":"string","maxLength":256},
                    "after":{
                        "type":"object",
                        "required":["updated_at","id"],
                        "properties":{
                            "updated_at":{"type":"string","format":"date-time"},
                            "id":{"type":"string"}
                        },
                        "additionalProperties":false
                    }
                })),
                papercut_action_schema("dump", &[], json!({
                    "scope":{"type":"string","enum":["project","all"],"default":"project"}
                })),
                papercut_action_schema("report", &["title"], json!({
                    "title":{"type":"string"},
                    "description":{"type":"string","default":""}
                })),
                papercut_action_schema("upvote", &["papercut_id"], json!({
                    "papercut_id":{"type":"string"},
                    "note":{"type":"string"}
                })),
                papercut_action_schema("remove", &["papercut_id","resolution"], json!({
                    "papercut_id":{"type":"string"},
                    "resolution":{"type":"string"}
                })),
                papercut_action_schema(
                    "respond",
                    &["review_id","reporter_token","decision","response"],
                    json!({
                        "review_id":{"type":"string"},
                        "reporter_token":{"type":"string"},
                        "decision":{"type":"string","enum":["approve","reject"]},
                        "response":{"type":"string"}
                    })
                )
            ]
        }
    })
}

fn sos_tool_definition() -> Value {
    json!({
        "name":"nexus_sos",
        "description":"Report complex problems that need help, inspect current aggregate issues and their source reports, dump retained SOS history, or resolve and deprecate aggregate issues. Nexus binds caller and project context and triages reports asynchronously.",
        "inputSchema":{
            "type":"object",
            "oneOf":[
                action_schema("list", &[], json!({})),
                action_schema("get", &["issue_id"], json!({
                    "issue_id":{"type":"string"}
                })),
                action_schema("dump", &[], json!({
                    "scope":{"type":"string","enum":["project","all"],"default":"project"}
                })),
                action_schema(
                    "report",
                    &["message","type","environment","version","model","trace"],
                    json!({
                        "message":{"type":"string"},
                        "type":{"type":"string"},
                        "environment":{"type":"string"},
                        "version":{"type":"string"},
                        "model":{"type":"string"},
                        "trace":{"type":"string"},
                        "metadata":{"type":"object","additionalProperties":true,"default":{}}
                    })
                ),
                action_schema("resolve", &["issue_id","resolution"], json!({
                    "issue_id":{"type":"string"},
                    "resolution":{"type":"string"}
                })),
                action_schema("deprecate", &["issue_id","reason"], json!({
                    "issue_id":{"type":"string"},
                    "reason":{"type":"string"}
                }))
            ]
        }
    })
}

fn papercut_action_schema(
    action: &str,
    required_fields: &[&str],
    action_properties: Value,
) -> Value {
    action_schema(action, required_fields, action_properties)
}

fn action_schema(action: &str, required_fields: &[&str], action_properties: Value) -> Value {
    let mut properties = json!({
        "action":{"type":"string","const":action}
    })
    .as_object()
    .expect("static common action properties are an object")
    .clone();
    properties.extend(
        action_properties
            .as_object()
            .expect("static action properties are an object")
            .clone(),
    );
    let required = std::iter::once("action")
        .chain(required_fields.iter().copied())
        .collect::<Vec<_>>();
    json!({
        "type":"object",
        "required":required,
        "properties":properties,
        "additionalProperties":false
    })
}

fn json_array(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        _ => unreachable!("static tool definitions are always JSON arrays"),
    }
}
