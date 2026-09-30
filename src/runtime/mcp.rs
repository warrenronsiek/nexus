// @feature runtime
// @spec docs/features/runtime.md
// @entrypoint serve_stdio
// @boundary dynamic-json
use super::daemon;
use crate::config::LoadedConfig;
use crate::coordination::api::ServiceRequest;
use crate::coordination::domain::HookResponse;
use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

const PROTOCOL_VERSION: &str = "2025-06-18";

/// Answers each request on its own task. Hosts multiplex hooks from many threads and
/// parallel tool calls over one connection, so one slow call must not queue the rest past
/// their host timeouts.
pub async fn serve_stdio(loaded: LoadedConfig, explicit_config: Option<&Path>) -> Result<()> {
    let loaded = Arc::new(loaded);
    let explicit_config = explicit_config.map(Path::to_path_buf);
    let (responses, outgoing) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(write_responses(outgoing));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let (loaded, explicit_config, responses) =
            (loaded.clone(), explicit_config.clone(), responses.clone());
        tokio::spawn(async move {
            let response = respond(&loaded, explicit_config.as_deref(), id, request).await;
            let _ = responses.send(response);
        });
    }
    drop(responses);
    writer.await?
}

/// Owns stdout so concurrently completed responses are written as whole lines.
async fn write_responses(mut outgoing: mpsc::UnboundedReceiver<Value>) -> Result<()> {
    let mut stdout = tokio::io::stdout();
    while let Some(response) = outgoing.recv().await {
        let mut line = serde_json::to_vec(&response)?;
        line.push(b'\n');
        stdout.write_all(&line).await?;
        stdout.flush().await?;
    }
    Ok(())
}

async fn respond(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    id: Value,
    request: Value,
) -> Value {
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    match method {
        "initialize" => {
            json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":PROTOCOL_VERSION,"capabilities":{"tools":{},"resources":{}},"serverInfo":{"name":"nexus","version":env!("CARGO_PKG_VERSION")}}})
        }
        "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}),
        "tools/list" => json!({"jsonrpc":"2.0","id":id,"result":{"tools":tool_definitions()}}),
        "tools/call" => tool_call(loaded, explicit_config, id, params).await,
        "resources/list" => json!({"jsonrpc":"2.0","id":id,"result":{"resources":[
            {"uri":"nexus://status","name":"Nexus status","mimeType":"application/json"},
            {"uri":"nexus://sessions","name":"Active agent sessions","mimeType":"application/json"},
            {"uri":"nexus://claims","name":"Active advisory claims","mimeType":"application/json"},
            {"uri":"nexus://conflicts","name":"Open coordination conflicts","mimeType":"application/json"},
            {"uri":"nexus://events","name":"Recent Nexus events","mimeType":"application/json"}
        ]}}),
        "resources/read" => resource_read(loaded, explicit_config, id, params).await,
        _ => {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("method not found: {method}")}})
        }
    }
}

async fn tool_call(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    id: Value,
    params: Value,
) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let method = name.strip_prefix("nexus_").unwrap_or(name);
    let lifecycle = ServiceRequest::is_lifecycle_method(method);
    let mut result = match ServiceRequest::decode(method, arguments) {
        Ok(request) if lifecycle => {
            daemon::lifecycle_request(loaded, explicit_config, &request).await
        }
        Ok(request) => daemon::ensure_and_request(loaded, explicit_config, &request)
            .await
            .unwrap_or_else(|error| json!({"ok":false,"error":error.to_string()})),
        Err(error) if lifecycle => serde_json::to_value(HookResponse::fail_open(error)).unwrap(),
        Err(error) => json!({"ok":false,"error":error.to_string()}),
    };
    add_pre_tool_context(method, &mut result);
    let text = hook_text(method, &result);
    json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":serde_json::to_string(&text).unwrap()}],"structuredContent":result,"isError":false}})
}

fn add_pre_tool_context(method: &str, result: &mut Value) {
    if method != "pre_tool_use" {
        return;
    }
    let Some(items) = result.get("advisories").and_then(Value::as_array) else {
        return;
    };
    if items.is_empty() {
        return;
    }
    let context = items
        .iter()
        .map(|item| {
            format!(
                "[{}] {} (session {})",
                item.get("severity")
                    .and_then(Value::as_str)
                    .unwrap_or("warning"),
                item.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("overlapping work detected"),
                item.get("other_session_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    result.as_object_mut().unwrap().insert(
        "hookSpecificOutput".into(),
        json!({
            "hookEventName":"PreToolUse",
            "additionalContext":format!("Nexus coordination advisories (informational only; you remain responsible for proceeding):\n{context}")
        }),
    );
}

fn hook_text(method: &str, result: &Value) -> Value {
    if !ServiceRequest::is_lifecycle_method(method) {
        return result.clone();
    }
    result
        .get("hookSpecificOutput")
        .map(|output| json!({"hookSpecificOutput":output}))
        .unwrap_or_else(|| json!({}))
}

async fn resource_read(
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
    id: Value,
    params: Value,
) -> Value {
    let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
    let (method, arguments) = match uri {
        "nexus://status" => ("status", json!({})),
        "nexus://sessions" => ("sessions", json!({"scope":"active"})),
        "nexus://claims" => ("claims", json!({"scope":"active"})),
        "nexus://conflicts" => ("conflicts", json!({"scope":"open"})),
        "nexus://events" => ("events", json!({"limit":100})),
        _ => {
            return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"unknown resource"}})
        }
    };
    let request = ServiceRequest::decode(method, arguments).expect("resource requests are valid");
    match daemon::ensure_and_request(loaded, explicit_config, &request).await {
        Ok(result) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"contents":[{"uri":uri,"mimeType":"application/json","text":serde_json::to_string_pretty(&result).unwrap()}]}})
        }
        Err(error) => {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":error.to_string()}})
        }
    }
}

fn tool_definitions() -> Value {
    let tool_hook_schema = json!({
        "type":"object",
        "required":["session_id","tool_use_id","tool_name"],
        "properties":{
            "session_id":{"type":"string"},"project_root":{"type":"string"},"agent":{"type":"string"},
            "tool_use_id":{"type":"string"},"tool_name":{"type":"string"},"tool_input":{},"tool_output":{},"error":{"type":"string"}
        },
        "additionalProperties":true
    });
    json!([
        {"name":"nexus_user_prompt","description":"Record a privacy-limited task synopsis for agent coordination.","inputSchema":{"type":"object","required":["session_id","prompt"],"properties":{"session_id":{"type":"string"},"project_root":{"type":"string"},"agent":{"type":"string"},"prompt":{"type":"string"}}}},
        {"name":"nexus_pre_tool_use","description":"Inspect an upcoming tool call and return non-blocking coordination advisories. Always permits execution.","inputSchema":tool_hook_schema},
        {"name":"nexus_post_tool_use","description":"Record successful tool completion. Never controls execution.","inputSchema":tool_hook_schema},
        {"name":"nexus_post_tool_failure","description":"Record failed tool completion. Never controls execution.","inputSchema":tool_hook_schema},
        {"name":"nexus_session_stop","description":"Mark a session stopped and release its advisory claims.","inputSchema":{"type":"object","required":["session_id"],"properties":{"session_id":{"type":"string"},"project_root":{"type":"string"},"agent":{"type":"string"}}}},
        {"name":"nexus_status","description":"Return daemon and projection counts.","inputSchema":{"type":"object"}},
        {"name":"nexus_sessions","description":"List agent sessions.","inputSchema":{"type":"object","properties":{"scope":{"type":"string","enum":["active","all"],"default":"active"}}}},
        {"name":"nexus_claims","description":"List advisory path claims.","inputSchema":{"type":"object","properties":{"project_id":{"type":"string"},"scope":{"type":"string","enum":["active","all"],"default":"active"}}}},
        {"name":"nexus_conflicts","description":"List detected overlaps.","inputSchema":{"type":"object","properties":{"project_id":{"type":"string"},"scope":{"type":"string","enum":["open","all"],"default":"open"}}}},
        {"name":"nexus_resolve","description":"Record an optional resolution chosen by an agent or user.","inputSchema":{"type":"object","required":["conflict_id"],"properties":{"conflict_id":{"type":"string"},"resolution":{"type":"string"}}}},
        {"name":"nexus_release","description":"Release a session's advisory claims.","inputSchema":{"type":"object","required":["session_id"],"properties":{"session_id":{"type":"string"},"path":{"type":"string"}}}},
        {"name":"nexus_analyze","description":"Explicitly ask the configured read-only Codex or Claude analyst to summarize one conflict and suggest optional resolutions.","inputSchema":{"type":"object","required":["conflict_id"],"properties":{"conflict_id":{"type":"string"}}}},
        {"name":"nexus_events","description":"Return recent coordination events.","inputSchema":{"type":"object","properties":{"project_id":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":10000}}}},
        {"name":"nexus_config","description":"Return resolved non-secret Nexus configuration and its hash.","inputSchema":{"type":"object"}}
    ])
}
