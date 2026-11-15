// @feature delivery-coordination
// @feature persistence
// @spec docs/features/delivery-coordination.md
// @spec docs/features/persistence.md
// @boundary test-fixtures: JSON fixtures are decoded through the public typed delivery contract.
#![cfg(feature = "sqlite")]
use nexus::{delivery::DeliveryAction, persistence::Store, work_history::WorkActor};
use serde_json::{json, Value};
fn actor(session: &str) -> WorkActor {
    WorkActor {
        agent: "codex".into(),
        session_id: session.into(),
        model_id: None,
        config_hash: "test".into(),
    }
}
fn command(store: &mut Store, project: &str, value: Value) -> anyhow::Result<Value> {
    let action: DeliveryAction = serde_json::from_value(value)?;
    Ok(serde_json::to_value(store.delivery_command(
        project,
        &actor("shepherd"),
        &action,
    )?)?)
}
fn revision(commit: char) -> Value {
    json!({"repository":"repo","object_format":"sha1","commit":commit.to_string().repeat(40)})
}
fn event(id: &str, body: Value) -> Value {
    json!({"id":id,"occurred_at":"2026-11-15T12:00:00Z","body":body})
}
fn record(store: &mut Store, events: Vec<Value>) -> anyhow::Result<Value> {
    command(
        store,
        "project",
        json!({"action":"record","observations":events}),
    )
}
fn contribution(id: &str, session: &str) -> Value {
    json!({"event":"contribution_registered","contribution":{"id":id,"repository":"repo","title":id,"origin":{"agent":"codex","session_id":session},"work_session_id":"work","worktree":"/tmp/work","branch":id,"base_revision":revision('a'),"head_revision":revision('b'),"features":["routing"],"rationale":"Preserve fallback behavior","constraints":["Do not discard retry state"]}})
}
fn mapping(id: &str, c: &str, start: u32) -> Value {
    json!({"event":"revision_mapped","mapping":{"id":id,"contribution_id":c,"revision":revision('c'),"source_revisions":[revision('b')],"operation":"squash","verification":"mechanically_verified","hunks":[{"path":"src/router.rs","old_path":"src/router.rs","old_start":start,"old_lines":3,"new_start":start,"new_lines":3}]}})
}
#[test]
fn squash_origin_lookup_preserves_hunk_authors_and_idempotent_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    let observations = vec![
        event("c1", contribution("one", "agent-one")),
        event("c2", contribution("two", "agent-two")),
        event("m1", mapping("map-one", "one", 10)),
        event("m2", mapping("map-two", "two", 40)),
    ];
    let first = record(&mut store, observations.clone()).unwrap();
    assert_eq!(first["inserted"], 4);
    let retry = record(&mut store, observations).unwrap();
    assert_eq!(retry["replayed"], 4);
    let query = json!({"action":"resolve_origin","query":{"revision":revision('c'),"path":"src/router.rs","line_start":11,"line_end":12}});
    let result = command(&mut store, "project", query.clone()).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["matches"][0]["contribution"]["origin"]["session_id"],
        "agent-one"
    );
    assert_eq!(result["qualification"], "exact");
    let isolated = command(&mut store, "other", query).unwrap();
    assert!(isolated["matches"].as_array().unwrap().is_empty());
    assert_eq!(isolated["qualification"], "unknown");
    let mut changed = contribution("one", "agent-one");
    changed["contribution"]["rationale"] = json!("changed");
    assert!(record(&mut store, vec![event("c1", changed)]).is_err());
}
#[test]
fn shepherd_handoff_is_compare_and_swap_and_keeps_history() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    record(&mut store,vec![event("pr",json!({"event":"pull_request_registered","pull_request":{"id":"pr-1","repository":"repo","provider":"github","number":12,"url":"https://example.test/pr/12","title":"Release","base":revision('a'),"head":revision('c'),"contribution_ids":[]}}))]).unwrap();
    let assign = |id: &str, expected: Option<&str>, session: &str| {
        event(
            id,
            json!({"event":"shepherd_assigned","subject":{"kind":"pull_request","id":"pr-1"},"expected_assignment":expected,"agent":{"agent":"codex","session_id":session},"reason":"handoff"}),
        )
    };
    record(&mut store, vec![assign("owner-one", None, "one")]).unwrap();
    assert!(record(&mut store, vec![assign("owner-two", None, "two")]).is_err());
    record(
        &mut store,
        vec![assign("owner-two", Some("owner-one"), "two")],
    )
    .unwrap();
    let result = command(
        &mut store,
        "project",
        json!({"action":"get","subject":{"kind":"pull_request","id":"pr-1"}}),
    )
    .unwrap();
    assert_eq!(result["record"]["shepherd"]["agent"]["session_id"], "two");
    assert_eq!(result["observations"].as_array().unwrap().len(), 3);
}
#[test]
fn origin_queries_reject_unanchored_ranges_and_conflicting_repository_identity() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    assert!(command(&mut store,"project",json!({"action":"resolve_origin","query":{"repository":"repo","path":"src/router.rs","line_start":11}})).is_err());
    assert!(command(
        &mut store,
        "project",
        json!({"action":"resolve_origin","query":{"repository":"other","revision":revision('c')}})
    )
    .is_err());
}
#[test]
fn stale_presence_and_overlapping_provenance_remain_qualified() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    record(&mut store,vec![event("c1",contribution("one","agent-one")),event("c2",contribution("two","agent-two")),event("m1",mapping("map-one","one",10)),event("m2",mapping("map-two","two",10)),event("presence",json!({"event":"agent_presence_observed","presence":{"agent":{"agent":"codex","session_id":"agent-one"},"reachability":"available","route":null,"observed_at":"2020-01-01T00:00:00Z"}}))]).unwrap();
    let found=command(&mut store,"project",json!({"action":"resolve_origin","query":{"revision":revision('c'),"path":"src/router.rs","line_start":11}})).unwrap();
    assert_eq!(found["qualification"], "ambiguous");
    assert_eq!(found["matches"][0]["presence"]["reachability"], "unknown");
}
#[test]
fn release_composition_remediation_and_portable_rebuild_preserve_original_actor() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("source.db")).unwrap();
    seed_release(&mut store);
    let inbox = repair_inbox(&mut store);
    assert_eq!(inbox["items"].as_array().unwrap().len(), 1);
    assert!(record(&mut store,vec![event("bad",json!({"event":"remediation_transitioned","remediation_id":"repair","expected_state":"requested","state":"resolved","reason":"fixed"}))]).is_err());
    record(&mut store,vec![event("start",json!({"event":"remediation_transitioned","remediation_id":"repair","expected_state":"requested","state":"in_progress","reason":"reproduced"})),event("finish",json!({"event":"remediation_transitioned","remediation_id":"repair","expected_state":"in_progress","state":"resolved","reason":"fixed","fixing_revision":revision('d')}))]).unwrap();
    let path = temp.path().join("snapshot.jsonl");
    store
        .write_streamed_snapshot(std::fs::File::create(&path).unwrap())
        .unwrap();
    let mut target = Store::open(&temp.path().join("target.db")).unwrap();
    target.import_streamed_snapshot(&path).unwrap();
    let repaired = command(
        &mut target,
        "project",
        json!({"action":"get","subject":{"kind":"remediation","id":"repair"}}),
    )
    .unwrap();
    assert_eq!(repaired["record"]["state"], "resolved");
    let inbox = repair_inbox(&mut target);
    assert!(inbox["items"].as_array().unwrap().is_empty());
    assert_eq!(
        repaired["observations"][0]["provenance"]["session_id"],
        "shepherd"
    );
    let origin = command(
        &mut target,
        "project",
        json!({"action":"resolve_origin","query":{"rollout_id":"release"}}),
    )
    .unwrap();
    assert_eq!(
        origin["matches"][0]["contribution"]["origin"]["session_id"],
        "agent-one"
    );
    assert!(target
        .list_projects()
        .unwrap()
        .iter()
        .any(|p| p.project_id == "project"));
}
#[test]
fn detail_exposes_shepherd_presence_without_changing_assignment() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    record(&mut store,vec![event("r",json!({"event":"rollout_registered","rollout":{"id":"release","repository":"repo","title":"Release","pull_request_ids":[],"contribution_ids":[],"environments":["development"]}})),event("s",json!({"event":"shepherd_assigned","subject":{"kind":"rollout","id":"release"},"expected_assignment":null,"agent":{"agent":"codex","session_id":"owner"},"reason":"watching release"})),event("p",json!({"event":"agent_presence_observed","presence":{"agent":{"agent":"codex","session_id":"owner"},"reachability":"unavailable","route":null,"observed_at":"2026-11-15T12:00:00Z"}}))]).unwrap();
    let result = command(
        &mut store,
        "project",
        json!({"action":"get","subject":{"kind":"rollout","id":"release"}}),
    )
    .unwrap();
    assert_eq!(
        result["record"]["shepherd_presence"]["reachability"],
        "unavailable"
    );
    assert_eq!(result["record"]["shepherd"]["event_id"], "s");
}

fn seed_release(store: &mut Store) {
    record(store,vec![event("c",contribution("one","agent-one")),event("r",json!({"event":"rollout_registered","rollout":{"id":"release","repository":"repo","title":"Release","pull_request_ids":[],"contribution_ids":["one"],"environments":["development","production"]}})),event("d",json!({"event":"deployment_recorded","deployment":{"id":"deploy","repository":"repo","rollout_id":"release","environment":"production","revision":revision('c'),"artifact_digest":"sha256:immutable","attempt":1,"outcome":"failed"}})),event("request",json!({"event":"remediation_requested","remediation":{"id":"repair","repository":"repo","title":"Fix router","recipient":{"agent":"codex","session_id":"agent-one"},"contribution_id":"one","deployment_id":"deploy","rollout_id":"release","evidence":["https://example.test/failure"],"request":"Restore retry state"}}))]).unwrap();
}
#[test]
fn deployment_completion_preserves_the_attempt_artifact_and_rejects_changed_outcomes() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    record(&mut store,vec![event("deploy",json!({"event":"deployment_recorded","deployment":{"id":"attempt","repository":"repo","rollout_id":"release","environment":"production","revision":revision('c'),"artifact_digest":"sha256:pinned","attempt":1,"outcome":"running"}}))]).unwrap();
    record(&mut store,vec![event("finish",json!({"event":"deployment_finished","deployment_id":"attempt","expected_outcome":"running","outcome":"succeeded"}))]).unwrap();
    let found = command(
        &mut store,
        "project",
        json!({"action":"get","subject":{"kind":"deployment","id":"attempt"}}),
    )
    .unwrap();
    assert_eq!(found["record"]["state"], "succeeded");
    assert_eq!(
        found["record"]["body"]["deployment"]["artifact_digest"],
        "sha256:pinned"
    );
    assert!(record(&mut store,vec![event("conflict",json!({"event":"deployment_finished","deployment_id":"attempt","expected_outcome":"running","outcome":"failed"}))]).is_err());
}
#[test]
fn origin_revision_lookup_intersects_the_selected_pull_request_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    record(&mut store,vec![event("one",contribution("one","one")),event("two",contribution("two","two")),event("m1",mapping("m1","one",10)),event("m2",mapping("m2","two",40)),event("pr",json!({"event":"pull_request_registered","pull_request":{"id":"selected","repository":"repo","provider":"github","number":3,"url":"https://example.test/pr/3","title":"Selected","base":revision('a'),"head":revision('c'),"contribution_ids":["one"]}}))]).unwrap();
    let found=command(&mut store,"project",json!({"action":"resolve_origin","query":{"revision":revision('c'),"pull_request_id":"selected"}})).unwrap();
    assert_eq!(found["matches"].as_array().unwrap().len(), 1);
    assert_eq!(found["matches"][0]["contribution"]["id"], "one");
}
#[test]
fn content_fingerprints_resolve_a_patch_across_changed_commit_identity() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    let digest = "f".repeat(64);
    let mut body = mapping("map", "one", 10);
    body["mapping"]["hunks"][0]["fingerprint"] = json!(digest);
    record(
        &mut store,
        vec![event("c", contribution("one", "agent")), event("m", body)],
    )
    .unwrap();
    let found = command(
        &mut store,
        "project",
        json!({"action":"resolve_origin","query":{"repository":"repo","fingerprint":digest}}),
    )
    .unwrap();
    assert_eq!(found["matches"][0]["contribution"]["id"], "one");
    assert_eq!(found["qualification"], "exact");
}

#[test]
fn target_filter_matches_rollout_environments_and_deployments() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    seed_release(&mut store);
    let selected = command(
        &mut store,
        "project",
        json!({"action":"list","query":{"target":"production"}}),
    )
    .unwrap();
    assert_eq!(selected["items"].as_array().unwrap().len(), 2);
}
#[test]
fn newer_pull_request_targets_replace_the_discovery_index() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    record(&mut store,vec![event("pr",json!({"event":"pull_request_registered","pull_request":{"id":"retargeted","repository":"repo","provider":"github","number":6,"url":"https://example.test/pr/6","title":"Retargeted","base":revision('a'),"head":revision('b'),"base_ref":"development","head_ref":"feature/old","contribution_ids":[]}})),event("observed",json!({"event":"pull_request_observed","pull_request_id":"retargeted","snapshot":{"observed_at":"2026-11-15T13:00:00Z","state":"open","base":revision('b'),"head":revision('c'),"base_ref":"staging","head_ref":"feature/new","checks":[]}}))]).unwrap();
    let old = command(
        &mut store,
        "project",
        json!({"action":"list","query":{"target":"development"}}),
    )
    .unwrap();
    assert!(old["items"].as_array().unwrap().is_empty());
    let current = command(
        &mut store,
        "project",
        json!({"action":"list","query":{"target":"staging","branch":"feature/new"}}),
    )
    .unwrap();
    assert_eq!(current["items"].as_array().unwrap().len(), 1);
    let previous_branch = command(
        &mut store,
        "project",
        json!({"action":"list","query":{"branch":"feature/old"}}),
    )
    .unwrap();
    assert!(previous_branch["items"].as_array().unwrap().is_empty());
}
#[test]
fn unanchored_repeated_patch_fingerprints_disclose_multiple_possible_authors() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    let digest = "d".repeat(64);
    let mut first = mapping("first", "one", 10);
    let mut second = mapping("second", "two", 40);
    first["mapping"]["hunks"][0]["fingerprint"] = json!(digest);
    second["mapping"]["hunks"][0]["fingerprint"] = json!(digest);
    record(
        &mut store,
        vec![
            event("one", contribution("one", "agent-one")),
            event("two", contribution("two", "agent-two")),
            event("first", first),
            event("second", second),
        ],
    )
    .unwrap();
    let found = command(
        &mut store,
        "project",
        json!({"action":"resolve_origin","query":{"repository":"repo","fingerprint":digest}}),
    )
    .unwrap();
    assert_eq!(found["matches"].as_array().unwrap().len(), 2);
    assert_eq!(found["qualification"], "ambiguous");
    assert!(found["matches"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["qualification"] == "ambiguous"));
}
fn registered_pr(id: &str, number: u64, contributions: &[&str]) -> Value {
    json!({"event":"pull_request_registered","pull_request":{"id":id,"repository":"repo","provider":"github","number":number,"url":format!("https://github.com/example/repo/pull/{number}"),"title":"Managed PR","base":revision('a'),"head":revision('b'),"base_ref":"development","head_ref":"work","contribution_ids":contributions}})
}
fn pushed_pr(id: &str) -> Value {
    json!({"event":"pull_request_observed","pull_request_id":id,"snapshot":{"observed_at":"2026-11-15T13:00:00Z","state":"open","base":revision('a'),"head":revision('c'),"base_ref":"staging","head_ref":"work","checks":[]}})
}
fn composition_recorded(expected: &str, head: char) -> Value {
    json!({"event":"pull_request_composition_recorded","pull_request_id":"managed","head":revision(head),"expected_composition":expected,"contribution_ids":["one","two"],"reason":"Include the second implementation after the latest push"})
}
fn push_ready_pr() -> Vec<Value> {
    vec![
        event("one", contribution("one", "agent-one")),
        event("two", contribution("two", "agent-two")),
        event("register", registered_pr("managed", 77, &["one"])),
        event(
            "owner",
            json!({"event":"shepherd_assigned","subject":{"kind":"pull_request","id":"managed"},"expected_assignment":null,"agent":{"agent":"codex","session_id":"owner"},"reason":"Track rollout"}),
        ),
        event("push", pushed_pr("managed")),
    ]
}
#[test]
fn provider_identity_prevents_two_shepherd_registries_for_one_actual_pr() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    record(
        &mut store,
        vec![
            event("register", registered_pr("original", 99, &[])),
            event("retarget", pushed_pr("original")),
        ],
    )
    .unwrap();
    let error = record(
        &mut store,
        vec![event("alias", registered_pr("different-id", 99, &[]))],
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("original"), "{error}");
    let found = command(
        &mut store,
        "project",
        json!({"action":"list","query":{"kind":"pull_request"}}),
    )
    .unwrap();
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    command(
        &mut store,
        "other-project",
        json!({"action":"record","observations":[event("other",registered_pr("other",99,&[]))]}),
    )
    .unwrap();
}
#[test]
fn pushed_pr_composition_compares_both_current_head_and_previous_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    record(&mut store, push_ready_pr()).unwrap();
    record(
        &mut store,
        vec![event("composition", composition_recorded("register", 'c'))],
    )
    .unwrap();
    let found = command(
        &mut store,
        "project",
        json!({"action":"get","subject":{"kind":"pull_request","id":"managed"}}),
    )
    .unwrap();
    assert_eq!(found["record"]["composition_event_id"], "composition");
    assert_eq!(
        found["record"]["body"]["pull_request"]["contribution_ids"],
        json!(["one", "two"])
    );
    assert_eq!(found["record"]["shepherd"]["event_id"], "owner");
    assert!(record(
        &mut store,
        vec![event(
            "stale-version",
            composition_recorded("register", 'c')
        )]
    )
    .is_err());
    assert!(record(
        &mut store,
        vec![event(
            "stale-head",
            composition_recorded("composition", 'b')
        )]
    )
    .is_err());
}
#[test]
fn portable_rebuild_retains_unique_provider_identity_and_current_composition() {
    let temp = tempfile::tempdir().unwrap();
    let mut source = Store::open(&temp.path().join("source.db")).unwrap();
    let mut observations = push_ready_pr();
    observations.push(event("composition", composition_recorded("register", 'c')));
    record(&mut source, observations).unwrap();
    let path = temp.path().join("snapshot.jsonl");
    source
        .write_streamed_snapshot(std::fs::File::create(&path).unwrap())
        .unwrap();
    let mut target = Store::open(&temp.path().join("target.db")).unwrap();
    target.import_streamed_snapshot(&path).unwrap();
    let found = command(
        &mut target,
        "project",
        json!({"action":"get","subject":{"kind":"pull_request","id":"managed"}}),
    )
    .unwrap();
    assert_eq!(found["record"]["composition_event_id"], "composition");
    assert_eq!(
        found["record"]["body"]["pull_request"]["contribution_ids"],
        json!(["one", "two"])
    );
    let error = record(
        &mut target,
        vec![event("duplicate", registered_pr("duplicate", 77, &[]))],
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("managed"), "{error}");
}

fn repair_inbox(store: &mut Store) -> Value {
    command(
        store,
        "project",
        json!({"action":"inbox","recipient":{"agent":"codex","session_id":"agent-one"}}),
    )
    .unwrap()
}
#[test]
fn rollout_origin_manifest_stays_frozen_when_its_source_pr_gains_contributions() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("delivery.db")).unwrap();
    let observations = vec![
        event("one", contribution("one", "agent-one")),
        event("two", contribution("two", "agent-two")),
        event("register", registered_pr("managed", 77, &["one"])),
        event(
            "frozen",
            json!({"event":"rollout_registered","rollout":{"id":"frozen","repository":"repo","title":"Frozen rollout","pull_request_ids":["managed"],"contribution_ids":["one"],"environments":["production"]}}),
        ),
        event(
            "unfrozen",
            json!({"event":"rollout_registered","rollout":{"id":"unfrozen","repository":"repo","title":"Unfrozen evidence","pull_request_ids":["managed"],"environments":["development"]}}),
        ),
        event("push", pushed_pr("managed")),
        event("composition", composition_recorded("register", 'c')),
    ];
    record(&mut store, observations).unwrap();
    let frozen = command(
        &mut store,
        "project",
        json!({"action":"resolve_origin","query":{"rollout_id":"frozen"}}),
    )
    .unwrap();
    assert_eq!(frozen["matches"].as_array().unwrap().len(), 1);
    assert_eq!(frozen["matches"][0]["contribution"]["id"], "one");
    assert!(frozen["gaps"].as_array().unwrap().is_empty());
    let unfrozen = command(
        &mut store,
        "project",
        json!({"action":"resolve_origin","query":{"rollout_id":"unfrozen"}}),
    )
    .unwrap();
    assert_eq!(unfrozen["matches"].as_array().unwrap().len(), 2);
    assert!(unfrozen["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|gap| gap == "rollout_manifest_missing"));
    assert!(unfrozen["matches"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["qualification"] == "candidates"));
}
