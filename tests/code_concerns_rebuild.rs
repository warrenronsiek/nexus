// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
// @boundary concern-rebuild-fixtures: controlled records exercise portable audit and authorization history.
#![cfg(feature = "sqlite")]
#[path = "support/code_concerns.rs"]
mod fixtures;
use fixtures::*;
use nexus::code_concerns::{ConcernAction, ConcernActor};
use nexus::persistence::Store;
use serde_json::json;

fn rejected_proposal(store: &mut Store) -> String {
    let mut other = report("target-report", "target", "two", "router");
    other["body"]["title"] = json!("Distinct target wording");
    let saved = call(
        store,
        json!({"action":"record","observations":[report("source-report","source","one","router"),other]}),
    );
    let token = saved["receipts"][0]["reporter_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let proposed = observation(
        "proposal",
        "source",
        json!({"event":"duplicate_proposed","target_concern_id":"target","rationale":"Similar symptoms warrant comparison"}),
    );
    call(store, json!({"action":"record","observations":[proposed]}));
    let response = observation(
        "rejected",
        "source",
        json!({"event":"duplicate_responded","proposal_id":"proposal","decision":"reject","reason":"The source reporter identifies distinct failure causes"}),
    );
    call(
        store,
        json!({"action":"respond","observation":response,"reporter_token":token}),
    );
    token
}

#[test]
fn snapshot_rebuild_retains_a_rejected_duplicate_decision_without_allowing_a_second_answer() {
    let (_source_directory, mut source) = fixture();
    let token = rejected_proposal(&mut source);
    let mut bytes = Vec::new();
    source.write_streamed_snapshot(&mut bytes).unwrap();
    let (directory, mut target) = fixture();
    let path = directory.path().join("concern-snapshot.jsonl");
    std::fs::write(&path, bytes).unwrap();
    target.import_streamed_snapshot(&path).unwrap();
    let second = observation(
        "second-answer",
        "source",
        json!({"event":"duplicate_responded","proposal_id":"proposal","decision":"approve","reason":"An old decision must not be silently replaced"}),
    );
    let action: ConcernAction = serde_json::from_value(
        json!({"action":"respond","observation":second,"reporter_token":token}),
    )
    .unwrap();
    let actor = ConcernActor {
        agent: "codex".into(),
        session_id: "caller".into(),
        model_id: None,
        config_hash: "test".into(),
    };
    assert!(target.concern_command("project", &actor, &action).is_err());
    assert_eq!(summary(&mut target, "source")["canonical_id"], "source");
    assert_eq!(
        call(&mut target, json!({"action":"dump"}))["observations"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn canonical_population_query_rejects_consolidated_tombstones() {
    for action in ["list", "dashboard"] {
        let unsupported = serde_json::from_value::<ConcernAction>(
            json!({"action":action,"query":{"status":"consolidated"}}),
        );
        assert!(
            unsupported.is_err(),
            "canonical population must reject tombstone status for {action}"
        );
        for status in ["open", "resolved"] {
            serde_json::from_value::<ConcernAction>(
                json!({"action":action,"query":{"status":status}}),
            )
            .unwrap();
        }
    }
}

#[test]
fn repository_scope_includes_every_explicit_area_of_a_shared_encounter() {
    let (_directory, mut store) = fixture();
    let mut shared = report("shared", "shared-concern", "attempt", "router");
    let areas = shared["body"]["encounter"]["areas"].as_array_mut().unwrap();
    let mut additional = areas[0].clone();
    additional["area"]["repository"] = json!("second-repo");
    areas.push(additional);
    call(
        &mut store,
        json!({"action":"record","observations":[shared]}),
    );
    for action in ["dashboard", "list"] {
        let result = call(
            &mut store,
            json!({"action":action,"query":{"repository":"second-repo"}}),
        );
        if action == "dashboard" {
            assert_eq!(result["counts"]["raw_observations"], 1);
            assert_eq!(result["counts"]["known_source_occurrences"], 1);
        } else {
            assert_eq!(result["items"][0]["id"], "shared-concern");
            assert_eq!(result["items"][0]["counts"]["raw_observations"], 1);
        }
    }
    let absent = call(
        &mut store,
        json!({"action":"dashboard","query":{"repository":"unrecorded-repo"}}),
    );
    assert_eq!(absent["counts"]["raw_observations"], 0);
}

fn resolved_return(concern: &str, area_id: &str, roles: &[&str]) -> Vec<serde_json::Value> {
    let mut original = report(&format!("{concern}-report"), concern, concern, area_id);
    original["body"]["encounter"]["areas"][0]["area"]["id"] = json!(area_id);
    let mut resolved = observation(
        &format!("{concern}-resolve"),
        concern,
        json!({"event":"resolved","reason":"Verified first remedy","evidence":[]}),
    );
    resolved["occurred_at"] = json!("2026-10-21T12:00:00Z");
    let mut returned = encounter(&format!("{concern}-return"), area_id);
    let mut attribution = returned["areas"][0].clone();
    attribution["area"]["id"] = json!(area_id);
    returned["areas"] = roles
        .iter()
        .map(|role| {
            let mut item = attribution.clone();
            item["relation"] = json!(role);
            item
        })
        .collect();
    let mut returned = observation(
        &format!("{concern}-returned"),
        concern,
        json!({"event":"encountered","encounter":returned}),
    );
    returned["occurred_at"] = json!("2026-10-22T12:00:00Z");
    vec![original, resolved, returned]
}

#[test]
fn return_rank_counts_an_encounter_once_when_an_area_has_two_recorded_roles() {
    let (_directory, mut store) = fixture();
    let mut records = resolved_return("one", "a-single", &["affected"]);
    records.extend(resolved_return(
        "two",
        "z-overlap",
        &["affected", "suspected_cause"],
    ));
    call(
        &mut store,
        json!({"action":"record","observations":records}),
    );
    let dashboard = call(
        &mut store,
        json!({"action":"dashboard","query":{"sort":"returns"}}),
    );
    assert_eq!(dashboard["areas"][0]["area"]["id"], "a-single");
    assert_eq!(dashboard["areas"][1]["area"]["id"], "z-overlap");
    for area in dashboard["areas"].as_array().unwrap() {
        assert_eq!(area["counts"]["returns_after_resolution"], 1);
    }
}
