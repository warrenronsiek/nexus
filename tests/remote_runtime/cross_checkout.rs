// @feature coordination
// @feature runtime
// @spec docs/features/coordination.md
// @spec docs/features/runtime.md
// @boundary dynamic-json: remote native tool fixture
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_checkout_hooks_follow_remote_binding_and_stop_all_visited_projects() {
    let service = RunningService::start(Vec::new()).await;
    let startup = initialized_checkout();
    let target = initialized_checkout();
    std::fs::write(
        target.path().join(".git/nexus.toml"),
        "schema_version=1\n[client]\nproject_id='target-project'\n",
    )
    .unwrap();
    let config = service.write_client_config("startup-project", 3600);
    let mut mcp = StdioMcp::spawn(&config, startup.path());
    mcp.pre_tool(startup.path(), "traveler", "startup-edit", "tracked.txt");
    mcp.pre_tool(target.path(), "peer", "target-edit", "tracked.txt");
    let result = mcp.call(
        "nexus_pre_tool_use",
        json!({
            "agent":"codex", "session_id":"traveler", "project_root":startup.path(),
            "tool_use_id":"cross-edit", "tool_name":"Bash",
            "tool_input":{"cmd":"touch tracked.txt", "workdir":target.path()}
        }),
    );
    assert_eq!(result["recorded"], true, "{result}");
    assert_eq!(
        result["advisories"].as_array().unwrap().len(),
        1,
        "{result}"
    );
    assert!(result["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("peer"));
    mcp.call(
        "nexus_session_stop",
        json!({"agent":"codex", "session_id":"traveler", "project_root":startup.path()}),
    );
    for project in ["startup-project", "target-project"] {
        let client = NexusClient::new(&service.client_config(project, 3600), None);
        let claims = client
            .request(
                &ServiceRequest::decode("claims", json!({"project_id":project,"scope":"active"}))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            !claims["claims"]
                .as_array()
                .unwrap()
                .iter()
                .any(|claim| claim["session_id"] == "traveler"),
            "{project}: {claims}"
        );
        if project == "target-project" {
            assert!(
                claims["claims"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|claim| claim["session_id"] == "peer"),
                "{claims}"
            );
        }
    }
}
