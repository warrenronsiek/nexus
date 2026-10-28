// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
// @boundary concern-storage-fixtures: decode controlled typed action inputs.
#![cfg(feature = "sqlite")]
#[path = "support/code_concerns.rs"]
mod fixtures;
use fixtures::*;
use nexus::code_concerns::{ConcernAction, ConcernActor};
use serde_json::{json, Value};

fn rejected(store: &mut nexus::persistence::Store, action: Value) {
    let action: ConcernAction = serde_json::from_value(action).unwrap();
    let actor = ConcernActor {
        agent: "codex".into(),
        session_id: "caller".into(),
        config_hash: "test".into(),
        model_id: None,
    };
    assert!(store.concern_command("project", &actor, &action).is_err());
}

#[test]
fn semantic_consolidation_requires_the_source_reporter_capability_and_retains_both_histories() {
    let (_directory, mut store) = fixture();
    let mut other = report("other", "target", "two", "router");
    other["body"]["title"] = json!("Another description");
    let saved = call(
        &mut store,
        json!({"action":"record","observations":[report("first","source","one","router"),other]}),
    );
    let token = saved["receipts"][0]["reporter_token"].as_str().unwrap();
    let proposed = observation(
        "proposal",
        "source",
        json!({"event":"duplicate_proposed","target_concern_id":"target","rationale":"Same observed root failure, different wording"}),
    );
    call(
        &mut store,
        json!({"action":"record","observations":[proposed]}),
    );
    let response = observation(
        "approval",
        "source",
        json!({"event":"duplicate_responded","proposal_id":"proposal","decision":"approve","reason":"Original reporter confirms both observations"}),
    );
    rejected(
        &mut store,
        json!({"action":"record","observations":[response.clone()]}),
    );
    rejected(
        &mut store,
        json!({"action":"respond","observation":response.clone(),"reporter_token":"wrong"}),
    );
    let accepted = call(
        &mut store,
        json!({"action":"respond","observation":response.clone(),"reporter_token":token}),
    );
    let replay = call(
        &mut store,
        json!({"action":"respond","observation":response,"reporter_token":token}),
    );
    assert_eq!(accepted["receipts"], replay["receipts"]);
    let dump = call(&mut store, json!({"action":"dump"}));
    assert!(!serde_json::to_string(&dump).unwrap().contains(token));
    assert_eq!(dump["observations"].as_array().unwrap().len(), 4);
    let source = summary(&mut store, "source");
    assert_eq!(source["canonical_id"], "target");
    assert_eq!(source["counts"]["raw_observations"], 2);
}

#[test]
fn summary_pages_fit_the_transport_budget_and_keep_complete_population_totals() {
    let (_directory, mut store) = fixture();
    for chunk in 0..2 {
        let mut records = Vec::new();
        for index in 0..32 {
            let number = chunk * 32 + index;
            let mut event = report(
                &format!("event-{number:02}"),
                &format!("concern-{number:02}"),
                &format!("attempt-{number}"),
                "router",
            );
            event["body"]["title"] = json!(format!("Distinct failure{number}"));
            event["body"]["description"] = json!("d".repeat(4000));
            event["body"]["encounter"]["areas"]=json!((0..6).map(|area|json!({"area":{"repository":"repo","kind":"subsystem","key":format!("{area}{}","k".repeat(1000)),"display_name":"n".repeat(400)},"relation":"affected"})).collect::<Vec<_>>());
            records.push(event)
        }
        call(
            &mut store,
            json!({"action":"record","observations":records}),
        );
    }
    let page = call(&mut store, json!({"action":"list","query":{"limit":64}}));
    assert_eq!(page["total"], 64);
    assert!(serde_json::to_vec(&page).unwrap().len() <= 768 * 1024);
    assert!(page["next_cursor"].is_string());
    assert_eq!(page["partial"], true);
}

#[test]
fn concern_artifacts_reuse_git_validation_and_changed_event_ids_roll_back_the_batch() {
    let (_directory, mut store) = fixture();
    let mut invalid = report("bad", "a", "one", "router");
    invalid["body"]["encounter"]["evidence"] = json!([{"kind":"artifact","description":"Evidence","verification":"supplied","backing":{"kind":"git_file","repository":"repo","commit":"short","object_format":"sha1","path":"../secret","entry_mode":"100644"}}]);
    rejected(
        &mut store,
        json!({"action":"record","observations":[invalid]}),
    );
    call(
        &mut store,
        json!({"action":"record","observations":[report("first","a","one","router")]}),
    );
    let mut changed = report("first", "a", "one", "router");
    changed["body"]["description"] = json!("changed immutable content");
    rejected(
        &mut store,
        json!({"action":"record","observations":[report("other","b","two","parser"),changed]}),
    );
    let page = call(&mut store, json!({"action":"list"}));
    assert_eq!(page["total"], 1);
}

#[test]
fn invalid_query_cursors_are_permanent_input_errors_and_corrupt_transfer_scalars_are_rejected() {
    let (_directory, mut source) = fixture();
    let actor = ConcernActor {
        agent: "codex".into(),
        session_id: "caller".into(),
        model_id: None,
        config_hash: "test".into(),
    };
    let action: ConcernAction =
        serde_json::from_value(json!({"action":"list","query":{"after":"not-a-cursor"}})).unwrap();
    let error = source
        .concern_command("project", &actor, &action)
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<nexus::code_concerns::ConcernRejection>(),
        Some(nexus::code_concerns::ConcernRejection::Invalid(_))
    ));
}

#[test]
fn corrupt_transfer_scalars_are_rejected_before_publishing_any_imported_history() {
    let (_directory, mut source) = fixture();
    call(
        &mut source,
        json!({"action":"record","observations":[report("first","a","one","router")]}),
    );
    let mut bytes = Vec::new();
    source.write_streamed_snapshot(&mut bytes).unwrap();
    let mut records = String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    for record in &mut records {
        if record["data"]["table"] == "concern_observations" {
            record["data"]["rows"][0]["effective_time"] = json!("1900-01-01T00:00:00.000000000Z");
        }
    }
    let (directory, mut target) = fixture();
    let path = directory.path().join("invalid.jsonl");
    let encoded = records
        .into_iter()
        .map(|v| format!("{}\n", serde_json::to_string(&v).unwrap()))
        .collect::<String>();
    std::fs::write(&path, encoded).unwrap();
    assert!(target.import_streamed_snapshot(&path).is_err());
    assert_eq!(call(&mut target, json!({"action":"list"}))["total"], 0);
}

#[test]
fn an_old_pending_proposal_cannot_redirect_an_already_consolidated_source() {
    let (_directory, mut store) = fixture();
    let mut target = report("target-report", "target", "two", "parser");
    target["body"]["title"] = json!("Target issue");
    let mut alternate = report("alternate-report", "alternate", "three", "worker");
    alternate["body"]["title"] = json!("Alternate issue");
    let saved = call(
        &mut store,
        json!({"action":"record","observations":[report("first","source","one","router"),target,alternate]}),
    );
    let token = saved["receipts"][0]["reporter_token"].as_str().unwrap();
    for (id, target) in [("proposal-1", "target"), ("proposal-2", "alternate")] {
        call(
            &mut store,
            json!({"action":"record","observations":[observation(id,"source",json!({"event":"duplicate_proposed","target_concern_id":target,"rationale":"Candidate overlap"}))]}),
        );
    }
    let approval = |id: &str, proposal: &str| {
        observation(
            id,
            "source",
            json!({"event":"duplicate_responded","proposal_id":proposal,"decision":"approve","reason":"Source reporter confirmed"}),
        )
    };
    call(
        &mut store,
        json!({"action":"respond","observation":approval("approved-1","proposal-1"),"reporter_token":token}),
    );
    rejected(
        &mut store,
        json!({"action":"respond","observation":approval("approved-2","proposal-2"),"reporter_token":token}),
    );
    assert_eq!(summary(&mut store, "source")["canonical_id"], "target");
}

#[test]
fn host_session_counts_keep_exact_agents_and_original_import_attribution() {
    let (_directory, mut store) = fixture();
    call(
        &mut store,
        json!({"action":"record","observations":[report("first","a","one","router")]}),
    );
    let other = ConcernActor {
        agent: "claude".into(),
        session_id: "caller".into(),
        model_id: None,
        config_hash: "test".into(),
    };
    let action:ConcernAction=serde_json::from_value(json!({"action":"record","observations":[observation("second","a",json!({"event":"encountered","encounter":encounter("one","router")}))]})).unwrap();
    store.concern_command("project", &other, &action).unwrap();
    assert_eq!(summary(&mut store, "a")["counts"]["host_sessions"], 2);
    let exported = call(&mut store, json!({"action":"export"}));
    let (_target_directory, mut target) = fixture();
    call(
        &mut target,
        json!({"action":"import","observations":exported["observations"]}),
    );
    assert_eq!(summary(&mut target, "a")["counts"]["host_sessions"], 2);
}
