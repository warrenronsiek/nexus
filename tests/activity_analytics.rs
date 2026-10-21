// @feature usage-analytics
// @spec docs/features/usage-analytics.md
// @boundary analytics-wire-fixtures
use nexus::coordination::api::ServiceRequest;
use nexus::{Config, LoadedConfig, NexusService};
use serde_json::{json, Value};

fn service(root: &std::path::Path) -> NexusService {
    NexusService::connect(
        LoadedConfig {
            config: Config::default(),
            sources: vec![],
            hash: "analytics-test".into(),
        },
        root.join("analytics.db").to_str().unwrap(),
    )
    .unwrap()
}

fn call(service: &NexusService, action: Value) -> Value {
    service
        .handle_remote(
            "analytics-project",
            ServiceRequest::decode("analytics", action).unwrap(),
        )
        .to_json()
}

#[test]
fn analytics_distinguishes_cli_activity_from_an_offered_unused_tool() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    call(
        &service,
        json!({"action":"inventory", "session_id":"s1", "agent":"pi", "scope":"host_tools", "resources":[{
            "name":"github.checks", "kind":"tool", "origin":"host_tool", "exposure":"exposed", "description":"Inspect CI checks", "activities":["ci.inspect"]
        }]}),
    );
    let args = json!({"session_id":"s1", "agent":"pi", "tool_use_id":"call1", "tool_name":"Bash", "tool_input":{"command":"gh pr checks 42"}});
    for _ in 0..2 {
        service.handle_remote(
            "analytics-project",
            ServiceRequest::decode("post_tool_use", args.clone()).unwrap(),
        );
    }
    let result = call(&service, json!({"action":"summary"}));
    assert_eq!(result["observed_calls"], 1);
    assert_eq!(result["identified_actions"], 1);
    assert_eq!(result["activities"][0]["category_id"], "ci.inspect");
    assert_eq!(result["activities"][0]["calls"], 1);
    let catalog = call(&service, json!({"action":"catalog"}));
    let tool = catalog["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "github.checks")
        .unwrap();
    assert_eq!(tool["offered_sessions"], 1);
    assert_eq!(tool["calls"], 0);
    let gaps = call(&service, json!({"action":"opportunities"}));
    assert_eq!(gaps["opportunities"][0]["category_id"], "ci.inspect");
    assert_eq!(
        gaps["opportunities"][0]["supplied_alternatives"][0],
        "github.checks"
    );
}

#[test]
fn inventory_updates_preserve_the_fact_that_a_tool_was_offered() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    for exposure in ["exposed", "discoverable"] {
        let result = call(
            &service,
            json!({"action":"inventory","agent":"pi","session_id":"inventory-session","scope":"host_tools","resources":[{"name":"checks","kind":"tool","origin":"host_tool","exposure":exposure}]}),
        );
        assert_eq!(result["ok"], true, "{result}");
    }
    let catalog = call(&service, json!({"action":"catalog"}));
    assert_eq!(catalog["capabilities"][0]["offered_sessions"], 1);
    assert_eq!(catalog["capabilities"][0]["discoverable_sessions"], 1);
}

#[test]
fn analytics_classifier_limits_are_validated() {
    let mut config = Config::default();
    config.analytics.classification_timeout_seconds = 0;
    assert!(config.validate().is_err());
}

#[test]
fn orchestration_wrappers_do_not_duplicate_activity_counts() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    for (id, parent, command) in [
        ("outer", None, "python -c 'orchestrate()'"),
        ("child", Some("outer"), "gh pr checks"),
    ] {
        let args = json!({"session_id":"nested","agent":"pi","tool_use_id":id,"parent_tool_use_id":parent,"tool_name":"Bash","tool_input":{"command":command}});
        service.handle_remote(
            "analytics-project",
            ServiceRequest::decode("post_tool_use", args).unwrap(),
        );
    }
    let result = call(&service, json!({"action":"summary"}));
    assert_eq!(result["observed_calls"], 2);
    assert_eq!(result["identified_actions"], 1);
    assert_eq!(result["activities"].as_array().unwrap().len(), 1);
}

#[test]
fn activity_details_return_paged_implementations_without_inputs() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    service.handle_remote("analytics-project",ServiceRequest::decode("post_tool_use",json!({"session_id":"detail","agent":"pi","tool_use_id":"one","tool_name":"Bash","tool_input":{"command":"gh pr checks private-canary"}})).unwrap());
    let result = call(
        &service,
        json!({"action":"examples","category_id":"ci.inspect"}),
    );
    assert_eq!(result["examples"][0]["name"], "gh pr checks");
    assert_eq!(result["examples"][0]["tool_name"], "Bash");
    assert!(!result.to_string().contains("private-canary"));
}

#[test]
fn exact_host_tool_use_is_counted_even_when_it_resolves_to_a_cli_activity() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    call(
        &service,
        json!({"action":"inventory","session_id":"s","agent":"pi","scope":"host_tools","resources":[{"name":"Bash","kind":"tool","origin":"host_tool","exposure":"exposed"}]}),
    );
    service.handle_remote("analytics-project",ServiceRequest::decode("post_tool_use",json!({"session_id":"s","agent":"pi","tool_use_id":"one","tool_name":"Bash","tool_input":{"command":"git status"}})).unwrap());
    let catalog = call(&service, json!({"action":"catalog"}));
    let bash = catalog["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Bash")
        .unwrap();
    assert_eq!(bash["calls"], 1);
    let examples = call(
        &service,
        json!({"action":"examples","resource_key":bash["resource_key"]}),
    );
    assert_eq!(examples["examples"][0]["name"], "git status");
    let summary = call(&service, json!({"action":"summary"}));
    assert_eq!(summary["identified_actions"], 1);
}

#[test]
fn opportunities_do_not_offer_tools_from_a_different_session() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    call(
        &service,
        json!({"action":"inventory","session_id":"other","agent":"pi","scope":"host_tools","resources":[{"name":"checks","kind":"tool","origin":"host_tool","exposure":"exposed","activities":["ci.inspect"]}]}),
    );
    service.handle_remote("analytics-project",ServiceRequest::decode("post_tool_use",json!({"session_id":"s","agent":"pi","tool_use_id":"one","tool_name":"Bash","tool_input":{"command":"gh pr checks"}})).unwrap());
    let result = call(&service, json!({"action":"opportunities"}));
    assert_eq!(
        result["opportunities"][0]["supplied_alternatives"],
        json!([])
    );
}

#[test]
fn deterministic_categories_can_be_renamed_without_losing_their_counts() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    service.handle_remote("analytics-project",ServiceRequest::decode("post_tool_use",json!({"session_id":"s","agent":"pi","tool_use_id":"one","tool_name":"Bash","tool_input":{"command":"gh pr checks"}})).unwrap());
    let renamed = call(
        &service,
        json!({"action":"rename_category","id":"ci.inspect","name":"Inspect CI","definition":"Inspect continuous integration status"}),
    );
    assert_eq!(renamed["ok"], true, "{renamed}");
    let summary = call(&service, json!({"action":"summary"}));
    assert_eq!(summary["activities"][0]["category_id"], "ci.inspect");
    assert_eq!(summary["activities"][0]["name"], "Inspect CI");
    assert_eq!(summary["activities"][0]["calls"], 1);
}

#[test]
fn a_started_call_remains_visible_when_no_completion_arrives() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    service.handle_remote("analytics-project",ServiceRequest::decode("pre_tool_use",json!({"session_id":"s","agent":"pi","tool_use_id":"started","tool_name":"Bash","tool_input":{"command":"git status"}})).unwrap());
    let result = call(&service, json!({"action":"summary"}));
    assert_eq!(result["observed_calls"], 1);
    assert_eq!(result["identified_actions"], 0);
}

#[test]
fn catalog_pagination_includes_all_unused_resources() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let resources:Vec<_>=(0..70).map(|index|json!({"name":format!("tool-{index:03}"),"kind":"tool","origin":"host_tool","exposure":"discoverable"})).collect();
    call(
        &service,
        json!({"action":"inventory","session_id":"s","agent":"pi","scope":"host_tools","resources":resources}),
    );
    let mut offset = 0;
    let mut names = Vec::new();
    loop {
        let page = call(&service, json!({"action":"catalog","offset":offset}));
        for item in page["capabilities"].as_array().unwrap() {
            assert_eq!(item["calls"], 0);
            names.push(item["name"].as_str().unwrap().to_owned());
        }
        match page["next_offset"].as_u64() {
            Some(next) => offset = next,
            None => break,
        }
    }
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 70);
}

#[test]
fn an_inventoried_tool_reuses_its_activity_mapping_without_reclassifying_each_call() {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    call(
        &service,
        json!({"action":"inventory","session_id":"s","agent":"pi","scope":"host_tools","resources":[{"name":"github.checks","kind":"tool","origin":"host_tool","exposure":"exposed","activities":["ci.inspect"]}]}),
    );
    service.handle_remote("analytics-project",ServiceRequest::decode("post_tool_use",json!({"session_id":"s","agent":"pi","tool_use_id":"one","tool_name":"github.checks","tool_input":{}})).unwrap());
    let summary = call(&service, json!({"action":"summary"}));
    assert_eq!(summary["activities"][0]["category_id"], "ci.inspect");
}
