// @feature runtime
// @spec docs/features/runtime.md
// @boundary authenticated-json-rpc
use super::{canonical_project_id, RemoteDispatcher, ServerState};
use crate::coordination::api::ServiceRequest;
use crate::runtime::mcp;
use anyhow::Result;
use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

const MCP_SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_MCP_SESSIONS: usize = 4_096;

#[derive(Clone)]
struct McpSession {
    project_id: String,
    project_root: Option<String>,
    agent: String,
    initialized: bool,
    last_seen: Instant,
}

#[derive(Clone)]
struct McpCaller {
    project_id: String,
    project_root: Option<String>,
    agent: Option<String>,
}

struct IncomingMcpRequest {
    value: Value,
    method: String,
    caller: McpCaller,
    session_id: Option<String>,
}

#[derive(Default)]
pub(super) struct McpSessionRegistry {
    sessions: Mutex<HashMap<String, McpSession>>,
}

struct McpHttpError {
    status: StatusCode,
    message: &'static str,
}

type McpHttpResult<T> = std::result::Result<T, McpHttpError>;

impl McpHttpError {
    fn new(status: StatusCode, message: &'static str) -> Self {
        Self { status, message }
    }

    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }

    fn bad_request(message: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn unavailable() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "MCP session registry is unavailable",
        )
    }
}

struct DirectInvoker {
    dispatcher: RemoteDispatcher,
}

#[async_trait]
impl mcp::ServiceInvoker for DirectInvoker {
    async fn invoke(&self, request: ServiceRequest, lifecycle: bool) -> Result<Value> {
        Ok(self
            .dispatcher
            .clone()
            .dispatch(request, None, lifecycle)
            .await
            .to_json())
    }
}

pub(super) async fn request(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = transport_rejection(&headers) {
        return response;
    }
    let request = match IncomingMcpRequest::parse(&headers, &body) {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    let result = if request.method == "initialize" {
        initialize_session(&state, request).await
    } else {
        dispatch_session_request(&state, request).await
    };
    result.unwrap_or_else(McpHttpError::into_response)
}

async fn initialize_session(
    state: &ServerState,
    request: IncomingMcpRequest,
) -> McpHttpResult<Response> {
    request.validate_initialize()?;
    let session_id = format!("mcp-http-{}", Uuid::new_v4());
    let session = request.caller.new_session();
    let context = session.invocation_context(&session_id)?;
    let id = request
        .value
        .get("id")
        .cloned()
        .expect("initialize id was validated");
    let invoker = DirectInvoker {
        dispatcher: RemoteDispatcher::new(state, session.project_id.clone()),
    };
    let response = mcp::respond(&invoker, &context, id, request.value).await;
    state.mcp_sessions.insert(session_id.clone(), session)?;
    Ok(mcp_response(StatusCode::OK, Some(response), &session_id))
}

async fn dispatch_session_request(
    state: &ServerState,
    request: IncomingMcpRequest,
) -> McpHttpResult<Response> {
    if request.method == "notifications/initialized" {
        handle_initialized_notification(state, &request)
    } else {
        dispatch_initialized_request(state, request).await
    }
}

fn handle_initialized_notification(
    state: &ServerState,
    request: &IncomingMcpRequest,
) -> McpHttpResult<Response> {
    let session_id = request
        .session_id
        .as_deref()
        .ok_or_else(|| McpHttpError::bad_request("mcp-session-id is required"))?;
    request.validate_initialized_notification()?;
    state
        .mcp_sessions
        .mark_initialized(session_id, &request.caller)?;
    Ok(mcp_response(StatusCode::ACCEPTED, None, session_id))
}

async fn dispatch_initialized_request(
    state: &ServerState,
    request: IncomingMcpRequest,
) -> McpHttpResult<Response> {
    let session_id = request
        .session_id
        .as_deref()
        .ok_or_else(|| McpHttpError::bad_request("mcp-session-id is required"))?;
    let session = state
        .mcp_sessions
        .initialized(session_id, &request.caller)?;
    if request.value.get("id").is_none() {
        return Ok(mcp_response(StatusCode::ACCEPTED, None, session_id));
    }
    let context = session.invocation_context(session_id)?;
    let id = request
        .value
        .get("id")
        .cloned()
        .expect("request id checked");
    let invoker = DirectInvoker {
        dispatcher: RemoteDispatcher::new(state, session.project_id),
    };
    let response = mcp::respond(&invoker, &context, id, request.value).await;
    Ok(mcp_response(StatusCode::OK, Some(response), session_id))
}

impl IncomingMcpRequest {
    fn parse(headers: &HeaderMap, body: &[u8]) -> McpHttpResult<Self> {
        let value = parse_json_rpc(body)?;
        let method = json_rpc_method(&value)?;
        if value.get("id").is_some_and(|id| !mcp::valid_request_id(id)) {
            return Err(McpHttpError::bad_request("invalid JSON-RPC request id"));
        }
        validate_protocol(headers, &method)?;
        Ok(Self {
            value,
            method,
            caller: McpCaller::from_headers(headers)?,
            session_id: header_value(headers, "mcp-session-id").map(str::to_owned),
        })
    }

    fn validate_initialize(&self) -> McpHttpResult<()> {
        let params = self.value.get("params").and_then(Value::as_object);
        let requested_version = params
            .and_then(|params| params.get("protocolVersion"))
            .and_then(Value::as_str);
        let capabilities = params
            .and_then(|params| params.get("capabilities"))
            .and_then(Value::as_object);
        let client_info = params
            .and_then(|params| params.get("clientInfo"))
            .and_then(Value::as_object);
        let client_name = client_info
            .and_then(|client_info| client_info.get("name"))
            .and_then(Value::as_str);
        let client_version = client_info
            .and_then(|client_info| client_info.get("version"))
            .and_then(Value::as_str);
        let valid = self.value.get("id").is_some()
            && requested_version.is_some_and(|version| !version.trim().is_empty())
            && capabilities.is_some()
            && client_name.is_some_and(|name| !name.trim().is_empty())
            && client_version.is_some_and(|version| !version.trim().is_empty())
            && self.session_id.is_none();
        valid
            .then_some(())
            .ok_or_else(|| McpHttpError::bad_request("invalid initialize request"))
    }

    fn validate_initialized_notification(&self) -> McpHttpResult<()> {
        self.value
            .get("id")
            .is_none()
            .then_some(())
            .ok_or_else(|| McpHttpError::bad_request("initialized must be a notification"))
    }
}

impl McpCaller {
    fn from_headers(headers: &HeaderMap) -> McpHttpResult<Self> {
        let project_id = header_value(headers, super::NEXUS_PROJECT_HEADER)
            .and_then(canonical_project_id)
            .ok_or_else(|| McpHttpError::bad_request("x-nexus-project-id is required"))?;
        let project_root = optional_context_header(headers, "x-nexus-project-root", 4_096)?;
        let agent = optional_context_header(
            headers,
            super::NEXUS_AGENT_HEADER,
            crate::coordination::api::MAX_AGENT_BYTES,
        )?;
        Ok(Self {
            project_id,
            project_root,
            agent,
        })
    }

    fn new_session(&self) -> McpSession {
        McpSession {
            project_id: self.project_id.clone(),
            project_root: self.project_root.clone(),
            agent: self.agent.clone().unwrap_or_else(|| "mcp-http".to_owned()),
            initialized: false,
            last_seen: Instant::now(),
        }
    }

    fn matches(&self, session: &McpSession) -> bool {
        self.project_id == session.project_id
            && self
                .project_root
                .as_ref()
                .is_none_or(|root| Some(root) == session.project_root.as_ref())
            && self
                .agent
                .as_ref()
                .is_none_or(|agent| agent == &session.agent)
    }
}

impl McpSession {
    fn invocation_context(&self, session_id: &str) -> McpHttpResult<mcp::McpInvocationContext> {
        mcp::McpInvocationContext::remote(
            session_id.to_owned(),
            self.project_root.clone(),
            self.agent.clone(),
        )
        .map_err(|_| McpHttpError::bad_request("invalid MCP caller context"))
    }
}

impl McpSessionRegistry {
    fn insert(&self, session_id: String, session: McpSession) -> McpHttpResult<()> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| McpHttpError::unavailable())?;
        prune_sessions(&mut sessions);
        evict_oldest_if_full(&mut sessions);
        sessions.insert(session_id, session);
        Ok(())
    }

    fn mark_initialized(&self, session_id: &str, caller: &McpCaller) -> McpHttpResult<()> {
        self.update(session_id, caller, |session| {
            session.initialized = true;
            Ok(())
        })
    }

    fn initialized(&self, session_id: &str, caller: &McpCaller) -> McpHttpResult<McpSession> {
        self.update(session_id, caller, |session| {
            if !session.initialized {
                return Err(McpHttpError::bad_request("MCP session is not initialized"));
            }
            Ok(session.clone())
        })
    }

    fn update<T>(
        &self,
        session_id: &str,
        caller: &McpCaller,
        update: impl FnOnce(&mut McpSession) -> McpHttpResult<T>,
    ) -> McpHttpResult<T> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| McpHttpError::unavailable())?;
        prune_sessions(&mut sessions);
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| McpHttpError::new(StatusCode::NOT_FOUND, "MCP session not found"))?;
        if !caller.matches(session) {
            return Err(McpHttpError::bad_request("MCP session context changed"));
        }
        session.last_seen = Instant::now();
        update(session)
    }

    fn remove(&self, session_id: &str) -> McpHttpResult<bool> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| McpHttpError::unavailable())?;
        Ok(sessions.remove(session_id).is_some())
    }
}

pub(super) async fn delete(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    if let Err(error) = validate_subsequent_protocol(&headers) {
        return error.into_response();
    }
    let Some(session_id) = header_value(&headers, "mcp-session-id") else {
        return (StatusCode::BAD_REQUEST, "mcp-session-id is required").into_response();
    };
    match state.mcp_sessions.remove(session_id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "MCP session not found").into_response(),
        Err(error) => error.into_response(),
    }
}

pub(super) async fn get(headers: HeaderMap) -> Response {
    match validate_subsequent_protocol(&headers) {
        Ok(()) => {
            let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
            response
                .headers_mut()
                .insert(header::ALLOW, HeaderValue::from_static("POST, DELETE"));
            response
        }
        Err(error) => error.into_response(),
    }
}

fn parse_json_rpc(body: &[u8]) -> McpHttpResult<Value> {
    match serde_json::from_slice::<Value>(body) {
        Ok(value @ Value::Object(_)) => (value.get("jsonrpc").and_then(Value::as_str)
            == Some("2.0"))
        .then_some(value)
        .ok_or_else(|| McpHttpError::bad_request("jsonrpc must be 2.0")),
        _ => Err(McpHttpError::bad_request("invalid JSON-RPC request")),
    }
}

fn json_rpc_method(value: &Value) -> McpHttpResult<String> {
    value
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| !method.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| McpHttpError::bad_request("JSON-RPC method is required"))
}

fn validate_protocol(headers: &HeaderMap, method: &str) -> McpHttpResult<()> {
    if method == "initialize" {
        Ok(())
    } else {
        validate_subsequent_protocol(headers)
    }
}

fn validate_subsequent_protocol(headers: &HeaderMap) -> McpHttpResult<()> {
    (header_value(headers, "mcp-protocol-version") == Some(mcp::PROTOCOL_VERSION))
        .then_some(())
        .ok_or_else(|| McpHttpError::bad_request("unsupported MCP protocol version"))
}

fn optional_context_header(
    headers: &HeaderMap,
    name: &str,
    max_len: usize,
) -> McpHttpResult<Option<String>> {
    super::optional_bounded_header(headers, name, max_len)
        .map_err(|()| McpHttpError::bad_request("invalid MCP caller context"))
}

fn transport_rejection(headers: &HeaderMap) -> Option<Response> {
    if !accepts_streamable_http(headers) {
        Some(
            (
                StatusCode::NOT_ACCEPTABLE,
                "accept must include application/json and text/event-stream",
            )
                .into_response(),
        )
    } else if !has_json_content_type(headers) {
        Some(
            (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "content type must be application/json",
            )
                .into_response(),
        )
    } else {
        None
    }
}

fn mcp_response(status: StatusCode, body: Option<Value>, session_id: &str) -> Response {
    let mut response = match body {
        Some(body) => (status, Json(body)).into_response(),
        None => status.into_response(),
    };
    response.headers_mut().insert(
        HeaderName::from_static("mcp-session-id"),
        HeaderValue::from_str(session_id).expect("generated MCP session ids are valid headers"),
    );
    response
}

fn accepts_streamable_http(headers: &HeaderMap) -> bool {
    let mut accepts_json = false;
    let mut accepts_events = false;
    for media_type in headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|value| value.trim().split(';').next().unwrap_or(""))
    {
        accepts_json |= media_type.eq_ignore_ascii_case("application/json");
        accepts_events |= media_type.eq_ignore_ascii_case("text/event-stream");
    }
    accepts_json && accepts_events
}

fn has_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

fn prune_sessions(sessions: &mut HashMap<String, McpSession>) {
    sessions.retain(|_, session| session.last_seen.elapsed() <= MCP_SESSION_TTL);
}

fn evict_oldest_if_full(sessions: &mut HashMap<String, McpSession>) {
    if sessions.len() < MAX_MCP_SESSIONS {
        return;
    }
    if let Some(oldest) = sessions
        .iter()
        .min_by_key(|(_, session)| session.last_seen)
        .map(|(id, _)| id.clone())
    {
        sessions.remove(&oldest);
    }
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initialize_request(params: Value) -> IncomingMcpRequest {
        IncomingMcpRequest {
            value: serde_json::json!({
                "jsonrpc":"2.0",
                "id":1,
                "method":"initialize",
                "params":params
            }),
            method: "initialize".into(),
            caller: McpCaller {
                project_id: "project-a".into(),
                project_root: Some("/tmp/project".into()),
                agent: Some("codex".into()),
            },
            session_id: None,
        }
    }

    #[test]
    fn streamable_http_accept_media_types_are_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("Application/JSON, Text/Event-Stream"),
        );

        assert!(accepts_streamable_http(&headers));
    }

    #[test]
    fn caller_agent_header_uses_the_persisted_identity_bound() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-nexus-project-id"),
            HeaderValue::from_static("project-a"),
        );
        headers.insert(
            HeaderName::from_static("x-nexus-agent"),
            HeaderValue::from_str(&"a".repeat(257)).unwrap(),
        );

        assert!(McpCaller::from_headers(&headers).is_err());
    }

    #[test]
    fn initialize_requires_capabilities_and_complete_client_info() {
        for params in [
            serde_json::json!({
                "protocolVersion":mcp::PROTOCOL_VERSION,
                "clientInfo":{"name":"test-client","version":"1.0"}
            }),
            serde_json::json!({
                "protocolVersion":mcp::PROTOCOL_VERSION,
                "capabilities":[],
                "clientInfo":{"name":"test-client","version":"1.0"}
            }),
            serde_json::json!({
                "protocolVersion":mcp::PROTOCOL_VERSION,
                "capabilities":{}
            }),
            serde_json::json!({
                "protocolVersion":mcp::PROTOCOL_VERSION,
                "capabilities":{},
                "clientInfo":{"name":"test-client"}
            }),
            serde_json::json!({
                "protocolVersion":mcp::PROTOCOL_VERSION,
                "capabilities":{},
                "clientInfo":{"name":"","version":"1.0"}
            }),
        ] {
            assert!(
                initialize_request(params.clone())
                    .validate_initialize()
                    .is_err(),
                "accepted {params}"
            );
        }

        assert!(initialize_request(serde_json::json!({
            "protocolVersion":mcp::PROTOCOL_VERSION,
            "capabilities":{},
            "clientInfo":{"name":"test-client","version":"1.0"}
        }))
        .validate_initialize()
        .is_ok());
    }

    #[test]
    fn streamable_http_rejects_non_string_non_integer_request_ids() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-nexus-project-id"),
            HeaderValue::from_static("project-a"),
        );

        for invalid_id in [
            serde_json::json!(null),
            serde_json::json!(true),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!(1.5),
        ] {
            let body = serde_json::to_vec(&serde_json::json!({
                "jsonrpc":"2.0",
                "id":invalid_id,
                "method":"initialize",
                "params":{}
            }))
            .unwrap();
            let Err(error) = IncomingMcpRequest::parse(&headers, &body) else {
                panic!("invalid request id was accepted: {invalid_id}");
            };
            assert_eq!(error.status, StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn unsupported_get_advertises_the_supported_methods() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("mcp-protocol-version"),
            HeaderValue::from_static(mcp::PROTOCOL_VERSION),
        );

        let response = get(headers).await;

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            response.headers().get(header::ALLOW),
            Some(&HeaderValue::from_static("POST, DELETE"))
        );
    }
}
