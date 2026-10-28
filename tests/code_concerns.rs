// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
// @boundary concern-test-input: typed actions are decoded from controlled wire fixtures.
#![cfg(feature = "sqlite")]
use serde_json::{json, Value};

#[path = "support/code_concerns.rs"]
mod fixtures;
use fixtures::*;

#[test]
fn immutable_retries_and_shared_sources_preserve_observations_without_inflating_occurrences() {
    let (_directory, mut store) = fixture();
    let first = report("event-1", "concern-1", "execution-1", "router");
    let saved = call(
        &mut store,
        json!({"action":"record","observations":[first.clone()]}),
    );
    let retried = call(
        &mut store,
        json!({"action":"record","observations":[first]}),
    );
    assert_eq!(
        saved["receipts"][0]["recorded_at"],
        retried["receipts"][0]["recorded_at"]
    );
    assert_eq!(retried["replayed"], 1);
    call(
        &mut store,
        json!({"action":"record","observations":[observation("event-2","concern-1",json!({"event":"encountered","encounter":encounter("execution-1","router")}))]}),
    );
    let detail = summary(&mut store, "concern-1");
    assert_eq!(detail["counts"]["raw_observations"], 2);
    assert_eq!(detail["counts"]["known_source_occurrences"], 1);
    assert_eq!(detail["counts"]["work_sessions"], 1);
    assert_eq!(detail["counts"]["host_sessions"], 1);
}

#[test]
fn exact_matching_uses_structured_areas_and_area_totals_deduplicate_shared_failures() {
    let (_directory, mut store) = fixture();
    call(
        &mut store,
        json!({"action":"record","observations":[report("first","a","shared-execution","router"),report("same-area","b","shared-execution","router"),report("other-area","c","shared-execution","parser")]}),
    );
    let page = call(&mut store, json!({"action":"list"}));
    assert_eq!(page["total"], 2);
    let duplicate = call(&mut store, json!({"action":"get","concern_id":"b"}));
    assert_eq!(duplicate["concern"]["summary"]["canonical_id"], "a");
    assert_eq!(duplicate["concern"]["summary"]["status"], "consolidated");
    let dashboard = call(&mut store, json!({"action":"dashboard"}));
    assert_eq!(dashboard["counts"]["raw_observations"], 3);
    assert_eq!(dashboard["counts"]["known_source_occurrences"], 1);
    assert_eq!(dashboard["areas"].as_array().unwrap().len(), 2);
}

#[test]
fn lifecycle_uses_failure_time_and_preserves_unknown_ordering_and_remedy_motivation() {
    let (_directory, mut store) = fixture();
    call(
        &mut store,
        json!({"action":"record","observations":[report("first","a","one","router")]}),
    );
    let mut resolution = observation(
        "resolved",
        "a",
        json!({"event":"resolved","reason":"Fixed fallback selection","evidence":[]}),
    );
    resolution["occurred_at"] = json!("2026-10-21T12:00:00Z");
    resolution["observed_at"] = resolution["occurred_at"].clone();
    call(
        &mut store,
        json!({"action":"record","observations":[resolution]}),
    );
    let mut old = observation(
        "old-log",
        "a",
        json!({"event":"encountered","encounter":encounter("old","router")}),
    );
    old["observed_at"] = json!("2026-10-22T12:00:00Z");
    call(&mut store, json!({"action":"record","observations":[old]}));
    let detail = summary(&mut store, "a");
    assert_eq!(detail["status"], "resolved");
    let mut unknown = observation(
        "unknown-log",
        "a",
        json!({"event":"encountered","encounter":{}}),
    );
    unknown["occurred_at"] = Value::Null;
    unknown["observed_at"] = json!("2026-10-22T13:00:00Z");
    call(
        &mut store,
        json!({"action":"record","observations":[unknown]}),
    );
    let detail = summary(&mut store, "a");
    assert_eq!(detail["status"], "resolved");
    assert_eq!(
        detail["lifecycle_qualification"],
        "unknown_recurrence_order"
    );
    let mut recurrence = observation(
        "returned",
        "a",
        json!({"event":"encountered","encounter":encounter("new","router"),"fix_relation":"another_branch"}),
    );
    recurrence["occurred_at"] = json!("2026-10-23T12:00:00Z");
    recurrence["observed_at"] = recurrence["occurred_at"].clone();
    let remedy = observation(
        "remedy",
        "a",
        json!({"event":"remedy_recorded","encounter_ids":["returned"],"what":"Try explicit mapping","why":"A local branch did not contain the first fix","outcome":"Uncertain","evidence":[]}),
    );
    call(
        &mut store,
        json!({"action":"record","observations":[recurrence,remedy]}),
    );
    let detail = summary(&mut store, "a");
    assert_eq!(detail["status"], "open");
    assert_eq!(detail["counts"]["raw_observations"], 4);
    assert_eq!(detail["counts"]["returns_after_resolution"], 1);
}

#[test]
fn portable_snapshot_preserves_concern_identity_history_and_capability_hashes() {
    let (_directory, mut source) = fixture();
    call(
        &mut source,
        json!({"action":"record","observations":[report("z-first","canonical","one","router"),report("a-alias","alias","two","router")]}),
    );
    let before = call(&mut source, json!({"action":"dump"}));
    let mut bytes = Vec::new();
    source.write_streamed_snapshot(&mut bytes).unwrap();
    let (target_directory, mut target) = fixture();
    let path = target_directory.path().join("snapshot.jsonl");
    std::fs::write(&path, bytes).unwrap();
    target.import_streamed_snapshot(&path).unwrap();
    let after = call(&mut target, json!({"action":"dump"}));
    assert_eq!(after, before);
    let detail = summary(&mut target, "alias");
    assert_eq!(detail["canonical_id"], "canonical");
}

#[test]
fn window_rankings_use_only_the_selected_population_and_detail_starts_with_recent_evidence() {
    let (_directory, mut store) = fixture();
    let mut a = report("a-first", "a", "a-old", "router");
    a["context"]["work_session_id"] = json!("old-a");
    let mut b = report("b-first", "b", "b-new", "router");
    b["body"]["title"] = json!("Different bug");
    b["occurred_at"] = json!("2026-10-23T12:00:00Z");
    b["observed_at"] = b["occurred_at"].clone();
    b["context"]["work_session_id"] = json!("new-b");
    call(&mut store, json!({"action":"record","observations":[a,b]}));
    let mut more = Vec::new();
    for (id, concern, day, work) in [
        ("old-a-2", "a", 20, "old-a2"),
        ("old-a-3", "a", 20, "old-a3"),
        ("new-a", "a", 23, "new-a"),
        ("new-b-2", "b", 23, "new-b2"),
    ] {
        let mut event = observation(
            id,
            concern,
            json!({"event":"encountered","encounter":encounter(id,"router")}),
        );
        event["occurred_at"] = json!(format!("2026-10-{day}T13:00:00Z"));
        event["observed_at"] = event["occurred_at"].clone();
        event["context"]["work_session_id"] = json!(work);
        more.push(event);
    }
    call(&mut store, json!({"action":"record","observations":more}));
    let recent = call(
        &mut store,
        json!({"action":"list","query":{"sort":"work_sessions","occurred_after":"2026-10-23T00:00:00Z"}}),
    );
    assert_eq!(recent["items"][0]["id"], "b");
    for index in 0..10 {
        let mut note = observation(
            &format!("note-{index:02}"),
            "a",
            json!({"event":"evidence_added","note":format!("evidence{index}"),"evidence":[]}),
        );
        note["observed_at"] = json!(format!("2026-10-24T12:{index:02}:00Z"));
        note["occurred_at"] = note["observed_at"].clone();
        call(&mut store, json!({"action":"record","observations":[note]}));
    }
    let detail = call(&mut store, json!({"action":"get","concern_id":"a"}));
    assert_eq!(detail["concern"]["history"][0]["input"]["id"], "note-09");
}

#[test]
fn detail_opens_the_newest_evidence_when_history_exceeds_one_page() {
    let (_directory, mut store) = fixture();
    call(
        &mut store,
        json!({"action":"record","observations":[report("first","a","one","router")]}),
    );
    for index in 0..10 {
        let mut note = observation(
            &format!("note-{index:02}"),
            "a",
            json!({"event":"evidence_added","note":format!("evidence{index}"),"evidence":[]}),
        );
        note["observed_at"] = json!(format!("2026-10-24T12:{index:02}:00Z"));
        note["occurred_at"] = note["observed_at"].clone();
        call(&mut store, json!({"action":"record","observations":[note]}));
    }
    let detail = call(&mut store, json!({"action":"get","concern_id":"a"}));
    assert_eq!(detail["concern"]["history"][0]["input"]["id"], "note-09");
}

#[test]
fn same_time_recurrence_is_qualified_independently_of_lexical_event_identity() {
    for (resolution_id, encounter_id, occurred_at, expected) in [
        (
            "z-resolution",
            "a-encounter",
            json!("2026-10-21T12:00:00Z"),
            "same_time_recurrence_order_unknown",
        ),
        (
            "a-resolution",
            "z-encounter",
            json!("2026-10-21T12:00:00Z"),
            "same_time_recurrence_order_unknown",
        ),
        (
            "z-resolution",
            "a-encounter",
            Value::Null,
            "unknown_recurrence_order",
        ),
        (
            "a-resolution",
            "z-encounter",
            Value::Null,
            "unknown_recurrence_order",
        ),
    ] {
        let (_directory, mut store) = fixture();
        let mut resolution = observation(
            resolution_id,
            "a",
            json!({"event":"resolved","reason":"Fixed","evidence":[]}),
        );
        resolution["occurred_at"] = json!("2026-10-21T12:00:00Z");
        resolution["observed_at"] = resolution["occurred_at"].clone();
        let mut recurrence = observation(
            encounter_id,
            "a",
            json!({"event":"encountered","encounter":{}}),
        );
        recurrence["occurred_at"] = occurred_at;
        recurrence["observed_at"] = resolution["observed_at"].clone();
        call(
            &mut store,
            json!({"action":"record","observations":[report("first","a","one","router"),resolution,recurrence]}),
        );
        let actual = summary(&mut store, "a");
        assert_eq!(actual["status"], "resolved");
        assert_eq!(
            actual["lifecycle_qualification"], expected,
            "{resolution_id}/{encounter_id}"
        );
        assert_eq!(actual["counts"]["returns_after_resolution"], 0);
    }
}
