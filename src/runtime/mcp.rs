// @feature code-concerns
// @spec docs/features/code-concerns.md
// @feature evals
// @spec docs/features/evals.md
// @feature runtime
// @feature usage-analytics
// @feature agent-memory
// @feature papercuts
// @feature sos
// @feature work-history
// @spec docs/features/runtime.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
// @spec docs/features/work-history.md
// @entrypoint serve_stdio
// @boundary dynamic-json
use super::bounded_io;
use crate::config::LoadedConfig;
use crate::coordination::api::ServiceRequest;
use crate::coordination::domain::HookResponse;
use crate::runtime::work_capture::CaptureReceipt;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufRead, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

use uuid::Uuid;

mod analytics_schema;
mod checkouts;
mod concern_reporting;
mod concerns_schema;
mod evals_schema;
mod git_intents;
mod hook_output;
mod reconciliation;
mod tool_definitions;
mod work_schema;
use hook_output::{add_hook_context, hook_text};
#[cfg(test)]
use reconciliation::MAX_ACTIVE_CLIENT_SESSIONS;
use reconciliation::{spawn_remote_reconciler, ConfiguredInvoker};
use tool_definitions::tool_definitions;

pub(crate) const PROTOCOL_VERSION: &str = "2025-06-18";
const MAX_IN_FLIGHT_STDIO_REQUESTS: usize = 32;
const MAX_QUEUED_STDIO_RESPONSES: usize = 32;

#[derive(Debug)]
pub(crate) struct McpInvocationContext {
    session_id: String,
    project_root: Option<String>,
    agent: String,
    /// Set for companions of Nexus-spawned providers; their lifecycle is never recorded.
    internal_process: bool,
}

impl McpInvocationContext {
    fn from_process(agent: &str) -> Result<Self> {
        if agent.trim().is_empty() {
            anyhow::bail!("MCP agent identity cannot be empty");
        }
        Ok(Self {
            session_id: format!("mcp-{}", Uuid::new_v4()),
            project_root: Some(std::env::current_dir()?.to_string_lossy().into_owned()),
            agent: agent.to_owned(),
            internal_process: super::internal_process::active(),
        })
    }

    pub(crate) fn remote(
        session_id: String,
        project_root: Option<String>,
        agent: String,
    ) -> Result<Self> {
        if session_id.trim().is_empty() {
            anyhow::bail!("MCP session identity cannot be empty");
        }
        if agent.trim().is_empty() {
            anyhow::bail!("MCP agent identity cannot be empty");
        }
        Ok(Self {
            session_id,
            project_root,
            agent,
            internal_process: false,
        })
    }
}

#[async_trait]
pub(crate) trait ServiceInvoker: Send + Sync {
    async fn invoke(&self, request: ServiceRequest, lifecycle: bool) -> Result<Value>;
    async fn reject_lifecycle(&self, _agent: &str) -> Option<CaptureReceipt> {
        None
    }
    async fn capture_health(&self) -> Result<Value> {
        Ok(
            json!({"ok":true,"scope":"unavailable","available":false,"reason":"local_capture_not_exposed"}),
        )
    }
}

/// Answers each request on its own task. Hosts multiplex hooks from many threads and
/// parallel tool calls over one connection, so one slow call must not queue the rest past
/// their host timeouts.
pub async fn serve_stdio(
    loaded: LoadedConfig,
    explicit_config: Option<&Path>,
    agent: &str,
) -> Result<()> {
    let loaded = Arc::new(loaded);
    let invocation_context = Arc::new(McpInvocationContext::from_process(agent)?);
    let explicit_config = explicit_config.map(Path::to_path_buf);
    let invoker = Arc::new(ConfiguredInvoker::from_loaded(
        loaded.clone(),
        explicit_config.clone(),
    ));
    let reconciler = spawn_remote_reconciler(
        loaded.config.coordination.reconcile_seconds,
        invoker.clone(),
    );
    let request_slots = Arc::new(Semaphore::new(MAX_IN_FLIGHT_STDIO_REQUESTS));
    let (responses, outgoing) = mpsc::channel::<QueuedResponse>(MAX_QUEUED_STDIO_RESPONSES);
    let writer = tokio::spawn(write_responses(outgoing));
    let mut input = BufReader::new(tokio::io::stdin());
    while let Some(frame) = next_stdio_frame(&mut input).await? {
        let Ok(request) = serde_json::from_slice::<Value>(&frame) else {
            continue;
        };
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let permit = request_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| anyhow::anyhow!("stdio MCP request gate closed"))?;
        let (invoker, invocation_context, responses) = (
            invoker.clone(),
            invocation_context.clone(),
            responses.clone(),
        );
        tokio::spawn(async move {
            let response = respond(invoker.as_ref(), &invocation_context, id, request).await;
            let _ = responses
                .send(QueuedResponse {
                    value: response,
                    _permit: permit,
                })
                .await;
        });
    }
    if let Some(reconciler) = reconciler {
        reconciler.abort();
    }
    drop(responses);
    writer.await?
}

async fn next_stdio_frame<R>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncBufRead + Unpin,
{
    loop {
        match bounded_io::read_json_line(reader).await {
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                eprintln!("Nexus skipped an oversized MCP frame; later requests remain available");
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    bounded_io::discard_json_line(reader),
                )
                .await
                .map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "oversized MCP frame did not end within 2 seconds",
                    )
                })??;
            }
            result => return result,
        }
    }
}

/// Owns stdout so concurrently completed responses are written as whole lines.
struct QueuedResponse {
    value: Value,
    _permit: OwnedSemaphorePermit,
}

async fn write_responses(mut outgoing: mpsc::Receiver<QueuedResponse>) -> Result<()> {
    let mut stdout = tokio::io::stdout();
    while let Some(response) = outgoing.recv().await {
        let mut line = serde_json::to_vec(&response.value)?;
        line.push(b'\n');
        stdout.write_all(&line).await?;
        stdout.flush().await?;
    }
    Ok(())
}

pub(crate) async fn respond(
    invoker: &dyn ServiceInvoker,
    invocation_context: &McpInvocationContext,
    id: Value,
    request: Value,
) -> Value {
    if !valid_request_id(&id) {
        return json!({
            "jsonrpc":"2.0",
            "id":null,
            "error":{"code":-32600,"message":"request id must be a string or integer"}
        });
    }
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    match method {
        "initialize" => {
            json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":PROTOCOL_VERSION,"capabilities":{"tools":{},"resources":{}},"serverInfo":{"name":"nexus","version":env!("CARGO_PKG_VERSION")}}})
        }
        "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}),
        "tools/list" => json!({"jsonrpc":"2.0","id":id,"result":{"tools":tool_definitions()}}),
        "tools/call" => tool_call(invoker, invocation_context, id, params).await,
        "resources/list" => json!({"jsonrpc":"2.0","id":id,"result":{"resources":[
            {"uri":"nexus://status","name":"Nexus status","mimeType":"application/json"},
            {"uri":"nexus://sessions","name":"Active agent sessions","mimeType":"application/json"},
            {"uri":"nexus://claims","name":"Active advisory claims","mimeType":"application/json"},
            {"uri":"nexus://conflicts","name":"Open coordination conflicts","mimeType":"application/json"},
            {"uri":"nexus://events","name":"Recent Nexus events","mimeType":"application/json"},
            {"uri":"nexus://papercuts","name":"Complete current-project papercut registry","mimeType":"application/json"},
            {"uri":"nexus://sos","name":"Complete current-project SOS registry","mimeType":"application/json"}
        ]}}),
        "resources/read" => resource_read(invoker, invocation_context, id, params).await,
        _ => {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("method not found: {method}")}})
        }
    }
}

pub(crate) fn valid_request_id(id: &Value) -> bool {
    match id {
        Value::String(_) => true,
        Value::Number(number) => number.is_i64() || number.is_u64(),
        _ => false,
    }
}

async fn tool_call(
    invoker: &dyn ServiceInvoker,
    invocation_context: &McpInvocationContext,
    id: Value,
    params: Value,
) -> Value {
    let internal_lifecycle = invocation_context.internal_process
        && params
            .get("name")
            .and_then(Value::as_str)
            .and_then(supported_tool_method)
            .is_some_and(ServiceRequest::is_lifecycle_method);
    if internal_lifecycle {
        unrecorded_internal_lifecycle(id)
    } else {
        service_tool_call(invoker, invocation_context, id, params).await
    }
}

async fn service_tool_call(
    invoker: &dyn ServiceInvoker,
    invocation_context: &McpInvocationContext,
    id: Value,
    params: Value,
) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return invalid_params(id, "tools/call requires a string name");
    };
    let Some(method) = supported_tool_method(name) else {
        return invalid_params(id, format!("unknown tool: {name}"));
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let lifecycle = ServiceRequest::is_lifecycle_method(method);
    if !arguments.is_object() && !lifecycle {
        return invalid_params(id, "tools/call arguments must be an object");
    }
    let arguments = bind_tool_context(method, arguments, invocation_context);
    let mut result = if method == "capture_health" {
        if !arguments
            .as_object()
            .is_some_and(|object| object.is_empty())
        {
            return invalid_params(id, "capture health takes no arguments");
        }
        invoker.capture_health().await.unwrap_or_else(|_| json!({"ok":true,"scope":"local_companion","available":false,"reason":"busy_or_unavailable"}))
    } else {
        match ServiceRequest::decode(method, arguments) {
            Ok(request) => invoker
                .invoke(request, lifecycle)
                .await
                .unwrap_or_else(|error| json!({"ok":false,"error":error.to_string()})),
            Err(error) if lifecycle => {
                rejected_lifecycle(invoker, &invocation_context.agent, error).await
            }
            Err(error) => return invalid_params(id, error.to_string()),
        }
    };
    add_hook_context(method, &mut result);
    let text = hook_text(method, &result);
    let is_error = tool_result_is_error(&result);
    json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":serde_json::to_string(&text).unwrap()}],"structuredContent":result,"isError":is_error}})
}

/// Nexus's own provider processes are not user sessions: no coordination, capture,
/// analytics, memory context, or guidance. The host still receives a permissive no-op.
fn unrecorded_internal_lifecycle(id: Value) -> Value {
    let result = serde_json::to_value(super::internal_process::unrecorded_hook_response())
        .expect("hook response serializes");
    json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":"{}"}],"structuredContent":result,"isError":false}})
}

async fn rejected_lifecycle(
    invoker: &dyn ServiceInvoker,
    agent: &str,
    error: anyhow::Error,
) -> Value {
    let mut result = serde_json::to_value(HookResponse::fail_open(error)).unwrap();
    if let Some(receipt) = invoker.reject_lifecycle(agent).await {
        result["work_capture"] = json!(receipt);
    }
    result
}

fn bind_tool_context(
    method: &str,
    mut arguments: Value,
    invocation_context: &McpInvocationContext,
) -> Value {
    enrich_memory_arguments(method, &mut arguments, invocation_context);
    if method == "papercuts" {
        enrich_papercut_arguments(&mut arguments, invocation_context);
    }
    if method == "sos" {
        enrich_sos_arguments(&mut arguments, invocation_context);
    }
    if method == "concerns" {
        if let Ok(command) = concern_reporting::prepare(arguments.clone()) {
            arguments = serde_json::to_value(command).expect("typed concern command serializable");
        }
    }
    if matches!(method, "evals" | "work" | "concerns") {
        arguments = json!({
            "context": {
                "session_id":invocation_context.session_id,
                "project_root":invocation_context.project_root,
                "agent":invocation_context.agent
            },
            "command":arguments
        });
    }
    arguments
}

fn supported_tool_method(name: &str) -> Option<&str> {
    let method = name.strip_prefix("nexus_")?;
    let advertised = tool_definitions()
        .as_array()?
        .iter()
        .any(|definition| definition.get("name").and_then(Value::as_str) == Some(name));
    // Preserve explicit maintenance clients' administrative calls outside agent discovery.
    // Browser inspection uses its separate closed read catalog and cannot call these tools.
    (advertised
        || matches!(
            method,
            "memory_status" | "memory_context" | "memory_invalidate" | "memory_consolidate"
        ))
    .then_some(method)
}

fn invalid_params(id: Value, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":message.into()}})
}

fn tool_result_is_error(result: &Value) -> bool {
    result.get("ok").and_then(Value::as_bool) == Some(false)
}

fn enrich_papercut_arguments(arguments: &mut Value, context: &McpInvocationContext) {
    enrich_issue_arguments(arguments, context, ModelFieldPolicy::Remove);
}

fn enrich_sos_arguments(arguments: &mut Value, context: &McpInvocationContext) {
    enrich_issue_arguments(arguments, context, ModelFieldPolicy::Preserve);
}

#[derive(Clone, Copy)]
enum ModelFieldPolicy {
    Remove,
    Preserve,
}

fn enrich_issue_arguments(
    arguments: &mut Value,
    context: &McpInvocationContext,
    model: ModelFieldPolicy,
) {
    let Some(arguments) = arguments.as_object_mut() else {
        return;
    };
    for field in ["project_root", "session_id", "agent", "turn_id"] {
        arguments.remove(field);
    }
    if matches!(model, ModelFieldPolicy::Remove) {
        arguments.remove("model");
    }
    if let Some(project_root) = &context.project_root {
        arguments.insert("project_root".into(), Value::String(project_root.clone()));
    }
    arguments.insert(
        "session_id".into(),
        Value::String(context.session_id.clone()),
    );
    arguments.insert("agent".into(), Value::String(context.agent.clone()));
}

fn enrich_memory_arguments(method: &str, arguments: &mut Value, context: &McpInvocationContext) {
    if !matches!(method, "memory_add" | "memory_search") {
        return;
    }
    let Some(arguments) = arguments.as_object_mut() else {
        return;
    };
    arguments.remove("project_root");
    if let Some(project_root) = &context.project_root {
        arguments.insert("project_root".into(), Value::String(project_root.clone()));
    }
    if method == "memory_add" {
        for field in ["session_id", "agent", "turn_id", "model"] {
            arguments.remove(field);
        }
        arguments.insert(
            "session_id".into(),
            Value::String(context.session_id.clone()),
        );
        arguments.insert("agent".into(), Value::String(context.agent.clone()));
    }
}

async fn resource_read(
    invoker: &dyn ServiceInvoker,
    invocation_context: &McpInvocationContext,
    id: Value,
    params: Value,
) -> Value {
    let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
    let Some((method, arguments)) = resource_request(uri, invocation_context) else {
        return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"unknown resource"}});
    };
    let request = ServiceRequest::decode(method, arguments).expect("resource requests are valid");
    match invoker.invoke(request, false).await {
        Ok(result) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"contents":[{"uri":uri,"mimeType":"application/json","text":serde_json::to_string_pretty(&result).unwrap()}]}})
        }
        Err(error) => {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":error.to_string()}})
        }
    }
}

fn resource_request(
    uri: &str,
    invocation_context: &McpInvocationContext,
) -> Option<(&'static str, Value)> {
    match uri {
        "nexus://status" => Some(("status", json!({}))),
        "nexus://sessions" => Some(("sessions", json!({"scope":"active"}))),
        "nexus://claims" => Some(("claims", json!({"scope":"active"}))),
        "nexus://conflicts" => Some(("conflicts", json!({"scope":"open"}))),
        "nexus://events" => Some(("events", json!({"limit":64}))),
        "nexus://papercuts" => Some((
            "papercuts",
            json!({
                "action":"dump",
                "scope":"project",
                "project_root":invocation_context.project_root,
                "session_id":invocation_context.session_id,
                "agent":invocation_context.agent
            }),
        )),
        "nexus://sos" => Some((
            "sos",
            json!({
                "action":"dump",
                "scope":"project",
                "project_root":invocation_context.project_root,
                "session_id":invocation_context.session_id,
                "agent":invocation_context.agent
            }),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod framing_tests;
#[cfg(test)]
mod tests;
