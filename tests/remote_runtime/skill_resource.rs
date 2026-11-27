// @feature agent-traces
// @feature runtime
// @spec docs/features/agent-traces.md
// @spec docs/features/runtime.md
// @boundary mcp-json
use super::support::{HttpMcpClient, RunningService};
use reqwest::StatusCode;
use serde_json::{json, Value};

#[tokio::test]
async fn streamable_http_mcp_exposes_the_bundled_trace_setup_skill() {
    const URI: &str = "nexus://skills/nexus-agent-traces";
    const MIME_TYPE: &str = "text/markdown";
    let service = RunningService::start(Vec::new()).await;
    let mut mcp = HttpMcpClient::new(&service, "shared");
    let read = json!({
        "jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {"uri": URI}
    });
    let unauthorized = reqwest::Client::new()
        .post(service.endpoint("/mcp"))
        .header("accept", "application/json, text/event-stream")
        .header("x-nexus-project-id", "shared")
        .json(&read)
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    mcp.establish().await;
    let listed = mcp
        .send(json!({"jsonrpc":"2.0","id":2,"method":"resources/list"}))
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let entry = listed["result"]["resources"]
        .as_array()
        .expect("expected a resource catalog")
        .iter()
        .find(|entry| entry["uri"] == URI)
        .expect("trace setup skill must appear in resources/list");
    assert_eq!(entry["mimeType"], MIME_TYPE);

    let response = mcp
        .send(read)
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let content = &response["result"]["contents"][0];
    assert_eq!(content["uri"], URI);
    assert_eq!(content["mimeType"], MIME_TYPE);
    assert_eq!(
        content["text"],
        include_str!("../../skills/nexus-agent-traces/SKILL.md")
    );
}
