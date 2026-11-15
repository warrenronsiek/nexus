// @feature delivery-coordination
// @feature persistence
// @spec docs/features/delivery-coordination.md
// @spec docs/features/persistence.md
// @boundary fixture-json: decode mapping identity fixtures through the typed delivery command boundary.
use super::*;

#[test]
fn independent_contributions_can_reuse_a_mapping_label_and_replay_its_journal() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("source.db")).unwrap();
    let observations = vec![
        event("one", contribution("one", "agent-one")),
        event("two", contribution("two", "agent-two")),
        event("map-one", mapping("initial", "one", 10)),
        event("map-two", mapping("initial", "two", 40)),
    ];
    record(&mut store, observations.clone()).unwrap();
    assert_eq!(record(&mut store, observations).unwrap()["replayed"], 4);
    let found = command(
        &mut store,
        "project",
        json!({"action":"resolve_origin","query":{"revision":revision('c')}}),
    )
    .unwrap();
    assert_eq!(found["matches"].as_array().unwrap().len(), 2);
    let path = temp.path().join("snapshot.jsonl");
    store
        .write_streamed_snapshot(std::fs::File::create(&path).unwrap())
        .unwrap();
    let mut target = Store::open(&temp.path().join("target.db")).unwrap();
    target.import_streamed_snapshot(&path).unwrap();
    let imported = command(
        &mut target,
        "project",
        json!({"action":"resolve_origin","query":{"revision":revision('c')}}),
    )
    .unwrap();
    assert_eq!(imported["matches"], found["matches"]);
}
fn crossed_mapping_label_observations() -> Vec<Value> {
    let mut one_later = mapping("right", "one", 20);
    one_later["mapping"]["revision"] = revision('d');
    let mut two_later = mapping("left", "two", 50);
    two_later["mapping"]["revision"] = revision('d');
    vec![
        event("one", contribution("one", "agent-one")),
        event("two", contribution("two", "agent-two")),
        event("one-current", mapping("left", "one", 10)),
        event("one-later", one_later),
        event("two-current", mapping("right", "two", 40)),
        event("two-later", two_later),
    ]
}
#[test]
fn origin_mapping_selection_qualifies_both_contribution_and_mapping_identity() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("source.db")).unwrap();
    record(&mut store, crossed_mapping_label_observations()).unwrap();
    let found = command(
        &mut store,
        "project",
        json!({"action":"resolve_origin","query":{"revision":revision('c')}}),
    )
    .unwrap();
    let matches = found["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 2);
    assert!(matches
        .iter()
        .all(|item| item["mapping"]["revision"] == revision('c')));
    assert_eq!(found["qualification"], "exact");
}
