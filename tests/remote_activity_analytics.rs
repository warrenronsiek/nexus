// @feature usage-analytics
// @feature runtime
// @spec docs/features/usage-analytics.md
// @spec docs/features/runtime.md
// @boundary remote-companion-provider-fixture
#[path = "support/executable.rs"]
mod executable;
#[allow(dead_code)]
#[path = "remote_runtime/support.rs"]
mod support;
use nexus::coordination::api::ServiceRequest;
use nexus::runtime::client::NexusClient;
use serde_json::json;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_companion_classifies_local_scripts_and_keeps_their_supplied_origin() {
    let service = support::RunningService::start(Vec::new()).await;
    let temp = tempfile::tempdir().unwrap();
    let command = temp.path().join("classifier");
    executable::write_executable(&command,"#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"category_id\":\"external.http\",\"name\":\"HTTP requests\",\"definition\":\"Interact with HTTP services\",\"description\":\"Query an HTTP service\"}'\n");
    let root = temp.path().join("checkout");
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::write(
        root.join("scripts/check.py"),
        "import requests\nrequests.get(endpoint)\n",
    )
    .unwrap();
    let mut loaded = service.client_config("analytics-remote", 3600);
    loaded.config.analytics.classification_enabled = true;
    loaded.config.analyst.provider = nexus::config::ModelProvider::Claude;
    loaded.config.analyst.claude.command = command.to_string_lossy().into_owned();
    let client = NexusClient::new(&loaded, None);
    let arguments = json!({"session_id":"remote-script","agent":"codex","project_root":root,"tool_use_id":"script-one","tool_name":"Bash","tool_input":{"command":"python scripts/check.py"}});
    for method in ["pre_tool_use", "post_tool_use"] {
        let response = client
            .lifecycle_request(&ServiceRequest::decode(method, arguments.clone()).unwrap())
            .await;
        assert_eq!(response["recorded"], true, "{response}");
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = client
            .request(
                &ServiceRequest::decode(
                    "analytics",
                    json!({"action":"examples","category_id":"external.http"}),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        if response["examples"].as_array().unwrap().len() == 1 {
            assert_eq!(response["examples"][0]["name"], "scripts/check.py");
            assert_eq!(response["examples"][0]["origin"], "repository");
            assert_eq!(response["examples"][0]["evidence"], "model");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "remote classification did not finish: {response}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
