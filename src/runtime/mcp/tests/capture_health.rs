// @feature runtime
// @feature work-history
// @spec docs/features/runtime.md
// @spec docs/features/work-history.md
use super::*;

fn isolated_invoker() -> (tempfile::TempDir, ConfiguredInvoker, McpInvocationContext) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = crate::Config::default();
    config.storage.database_path = temp.path().join("nexus.db");
    config.runtime.socket_path = temp.path().join("absent.sock");
    let loaded = Arc::new(LoadedConfig {
        config,
        hash: "health".into(),
        sources: Vec::new(),
    });
    let invoker = ConfiguredInvoker::from_loaded(loaded, None);
    let context = McpInvocationContext::remote("host".into(), None, "codex".into()).unwrap();
    (temp, invoker, context)
}

#[tokio::test]
async fn capture_health_tool_reports_caller_outbox_without_a_service() {
    let (_temp, invoker, context) = isolated_invoker();
    let response = tool_call(
        &invoker,
        &context,
        json!(1),
        json!({"name":"nexus_capture_health","arguments":{}}),
    )
    .await;
    let health = &response["result"]["structuredContent"];
    assert_eq!(health["ok"], true);
    assert_eq!(health["scope"], "local_companion");
    assert_eq!(health["available"], true);
    assert_eq!(health["work_capture"]["queued"], 0);
    assert!(health["work_capture"]["capture"]["offered"].is_u64());
}

#[tokio::test]
async fn malformed_lifecycle_arguments_retain_only_rejection_metadata_and_fail_open() {
    let (temp, invoker, context) = isolated_invoker();
    let private = "unretained-private-callback-input";
    let response = tokio::time::timeout(
        Duration::from_secs(1),
        tool_call(
            &invoker,
            &context,
            json!(1),
            json!({"name":"nexus_pre_tool_use","arguments":{
                "session_id":{"private":private},"agent":private,
                "tool_name":"Read","tool_input":{"private":private}
            }}),
        ),
    )
    .await
    .unwrap();
    let result = &response["result"];
    let hook = &result["structuredContent"];
    assert_eq!(hook["permitted"], true);
    assert_eq!(hook["recorded"], false);
    assert_eq!(result["isError"], false);
    assert_eq!(result["content"][0]["text"], "{}");
    assert_eq!(hook["work_capture"]["offered"], 1);
    assert!(matches!(
        hook["work_capture"]["status"].as_str(),
        Some("rejected" | "pending")
    ));

    let ledger = temp.path().join("work-outbox/.capture-health");
    for _ in 0..100 {
        if ledger.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let bytes = std::fs::read(ledger).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(private));
    let health: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(health["transport_rejected"], 1);
    assert_eq!(
        health["transport_rejection_reasons"]["invalid_host_event"],
        1
    );
    assert_eq!(health["last_adapter_host"], "codex");
    assert!(health["last_adapter_rejection_at"].is_string());
    assert_eq!(health["durably_queued"], 0);
}
