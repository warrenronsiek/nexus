// @feature delivery-coordination
// @feature persistence
// @spec docs/features/delivery-coordination.md
// @spec docs/features/persistence.md
// @boundary fixture-json: service records are exercised through typed request decoding.
use super::*;

#[test]
fn origin_falls_back_to_provider_pr_and_groups_activity_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    let mut first = contribution("one", "origin-thread");
    first["contribution"]["activity_id"] = json!("same-activity");
    let mut second = contribution("two", "origin-thread");
    second["contribution"]["activity_id"] = json!("same-activity");
    record(
        &mut store,
        vec![
            event("first", first),
            event("second", second),
            event(
                "pr",
                json!({"event":"pull_request_registered","pull_request":{
                    "id":"internal-pr-id","repository":"repo","provider":"github","number":3521,
                    "url":"https://example.test/pr/3521","title":"Release","base":revision('a'),
                    "head":revision('c'),"contribution_ids":["one","two"]
                }}),
            ),
        ],
    )
    .unwrap();
    let found = command(
        &mut store,
        "project",
        json!({"action":"resolve_origin","query":{
            "repository":"repo","revision_prefix":"cccccccc","pull_request_number":3521,
            "branch":"some-other-branch","feature":"routing"
        }}),
    )
    .unwrap();
    assert_eq!(found["matched_by"], "pull_request", "{found}");
    assert_eq!(found["groups"].as_array().unwrap().len(), 1, "{found}");
    assert_eq!(
        found["groups"][0]["evidence"].as_array().unwrap().len(),
        2,
        "{found}"
    );

    let by_commit = command(
        &mut store,
        "project",
        json!({"action":"resolve_origin","query":{
            "repository":"repo","revision_prefix":"bbbbbbbb"
        }}),
    )
    .unwrap();
    assert_eq!(by_commit["matched_by"], "revision", "{by_commit}");
    assert_eq!(
        by_commit["groups"].as_array().unwrap().len(),
        1,
        "{by_commit}"
    );
}

#[test]
fn remediation_resolves_origin_and_replays_through_the_recipient_inbox() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    record(
        &mut store,
        vec![event("one", contribution("one", "origin-thread"))],
    )
    .unwrap();
    let request = json!({"action":"remediate","request":{
        "id":"request-1","origin":{"repository":"repo","feature":"routing"},
        "title":"Investigate rollout failure","message":"Check the timeline backlog",
        "evidence":["https://example.test/checks/42"]
    }});
    let result = command(&mut store, "project", request.clone()).unwrap();
    assert_eq!(
        result["remediation"]["recipient"]["session_id"], "origin-thread",
        "{result}"
    );
    assert_eq!(result["state"], "requested", "{result}");
    assert_eq!(
        result["remediation"]["origin_evidence"]["matched_by"], "feature",
        "{result}"
    );

    assert_eq!(result["delivery"], "queued", "{result}");
    assert_eq!(
        command(&mut store, "project", request).unwrap()["remediation"]["id"],
        "request-1"
    );
    let inbox = command(
        &mut store,
        "project",
        json!({"action":"inbox","recipient":{
            "agent":"codex","session_id":"origin-thread"
        }}),
    )
    .unwrap();
    assert_eq!(inbox["items"].as_array().unwrap().len(), 1, "{inbox}");
}

#[test]
fn delivery_environment_assessment_uses_explicit_cross_repository_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    record(&mut store, vec![
        event("requirement", json!({"event":"release_requirements_recorded","requirements":{
            "id":"candidate","revision":revision('c'),"requires":[
                {"repository":"sv-tf","key":"APP_QUEUE_INTEGRATION_MAPPING_NAME"}
            ]
        }})),
        event("inventory", json!({"event":"environment_inventory_observed","inventory":{
            "id":"beta-inventory","repository":"sv-tf","environment":"beta","keys":[],"complete":true,
            "expires_at":"2099-01-01T00:00:00Z"
        }})),
        event("advisory", json!({"event":"environment_advisory_reported","advisory":{
            "id":"busy-db","environment":"beta","title":"Database contention",
            "description":"Timeline replay saturates the shared database","expires_at":"2099-01-01T00:00:00Z"
        }})),
    ]).unwrap();
    let response = command(
        &mut store,
        "project",
        json!({"action":"environment_context",
            "environment":"beta","candidate":revision('c')
        }),
    )
    .unwrap();
    assert_eq!(
        response["dependencies"][0]["status"], "missing",
        "{response}"
    );
    assert_eq!(response["advisories"][0]["id"], "busy-db", "{response}");
}

#[test]
fn failure_history_counts_distinct_prs_across_observations() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    let failing = |id: &str, pr: &str| {
        event(
            id,
            json!({
                "event":"pull_request_observed","pull_request_id":pr,"snapshot":{
                    "observed_at":"2026-11-15T12:00:00Z","state":"open","base":revision('a'),"head":revision('c'),
                    "checks":[{"id":id,"name":"decision-support","state":"failed","revision":revision('c')}]
                }
            }),
        )
    };
    let pr = |id: &str, number: u64| {
        event(
            id,
            json!({"event":"pull_request_registered","pull_request":{
                "id":id,"repository":"repo","provider":"github","number":number,
                "url":format!("https://example.test/pr/{number}"),"title":"Release","base":revision('a'),
                "head":revision('c')
            }}),
        )
    };
    record(
        &mut store,
        vec![
            pr("one", 1),
            pr("two", 2),
            failing("failure-a", "one"),
            failing("failure-b", "one"),
            failing("failure-c", "two"),
        ],
    )
    .unwrap();
    let response = command(
        &mut store,
        "project",
        json!({"action":"check_history",
            "repository":"repo","check":"decision-support","since":"2026-11-14T12:00:00Z"
        }),
    )
    .unwrap();
    assert_eq!(
        response["pull_request_ids"],
        json!(["one", "two"]),
        "{response}"
    );
}

#[test]
fn promotion_watch_reports_readiness_for_the_open_pull_request_head() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    record(&mut store, vec![
        event("pr", json!({"event":"pull_request_registered","pull_request":{
            "id":"promotion-pr","repository":"repo","provider":"github","number":42,
            "url":"https://github.com/example/repo/pull/42","title":"Promote to beta","base":revision('a'),"head":revision('c')
        }})),
        event("manifest", json!({"event":"release_requirements_recorded","requirements":{
            "id":"c-manifest","revision":revision('c'),
            "requires":[{"repository":"infra","key":"QUEUE_MAPPING"}]
        }})),
        event("watch", json!({"event":"watch_registered","watch":{
            "id":"beta-promotion","pull_request_id":"promotion-pr","repository_id":"repo",
            "source":{"owner":"example","repository":"repo","number":42},"interval_seconds":60,
            "promotion_environment":"beta"
        }})),
    ]).unwrap();
    let result = command(&mut store, "project", json!({"action":"watches"})).unwrap();
    let readiness = &result["watches"][0]["promotion_readiness"];
    assert_eq!(readiness["candidate"], revision('c'), "{result}");
    assert_eq!(readiness["requirements_id"], "c-manifest", "{result}");
    assert_eq!(readiness["unknown"], 1, "{result}");
}

#[test]
fn concern_feature_can_address_a_handoff_without_repeating_origin_selectors() {
    use nexus::code_concerns::{ConcernAction, ConcernActor, ConcernResponse};

    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    record(
        &mut store,
        vec![event("origin", contribution("one", "implementer"))],
    )
    .unwrap();
    let reporter = ConcernActor {
        agent: "codex".into(),
        session_id: "reporter".into(),
        model_id: None,
        config_hash: "fixture".into(),
    };
    let report: ConcernAction = serde_json::from_value(json!({"action":"record","observations":[{
        "id":"report", "concern_id":"router-concern","observed_at":"2026-11-15T12:00:00Z",
        "body":{"event":"reported","title":"Router retries failed","description":"Observed a retry failure",
            "encounter":{"areas":[{"area":{"repository":"repo","kind":"feature","key":"routing",
                "display_name":"Routing"},"relation":"affected"}]}}
    }]})).unwrap();
    let ConcernResponse::Recorded { receipts, .. } = store
        .concern_command("project", &reporter, &report)
        .unwrap()
    else {
        panic!("expected concern receipt")
    };
    let result = command(
        &mut store,
        "project",
        json!({"action":"remediate","request":{
            "concern_id":receipts[0].concern_id,"title":"Investigate retries",
            "message":"Review the reported retry failure"
        }}),
    )
    .unwrap();
    assert_eq!(result["remediation"]["concern_id"], receipts[0].concern_id);
    assert_eq!(
        result["remediation"]["recipient"]["session_id"],
        "implementer"
    );
}

#[test]
fn dated_delivery_registry_seeks_in_time_order_across_pages() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    let pr = |id: &str, number: u64, occurred_at: &str| {
        let mut item = event(
            id,
            json!({"event":"pull_request_registered","pull_request":{
                "id":id,"repository":"repo","provider":"github","number":number,
                "url":format!("https://example.test/pr/{number}"),"title":"Release","base":revision('a'),"head":revision('c')
            }}),
        );
        item["occurred_at"] = json!(occurred_at);
        item
    };
    record(
        &mut store,
        vec![
            pr("a-older", 1, "2026-11-14T12:00:00Z"),
            pr("z-newer", 2, "2026-11-15T12:00:00Z"),
        ],
    )
    .unwrap();
    let page = |store: &mut Store, after: Value| {
        command(
            store,
            "project",
            json!({
                "action":"list","query":{"kind":"pull_request","limit":1,
                    "updated_since":"2026-11-13T12:00:00Z","after":after}
            }),
        )
        .unwrap()
    };
    let first = page(&mut store, Value::Null);
    assert_eq!(first["items"][0]["subject"]["id"], "z-newer");
    let second = page(&mut store, first["next_cursor"].clone());
    assert_eq!(second["items"][0]["subject"]["id"], "a-older");
    assert_eq!(second["next_cursor"], Value::Null);
}
