// @feature delivery-coordination
// @feature persistence
// @spec docs/features/delivery-coordination.md
// @spec docs/features/persistence.md
// @boundary postgres-delivery-fixtures: explicit test-service access and JSON fixtures cross the public typed delivery contract.
#![cfg(feature = "postgres")]

use nexus::{
    delivery::{DeliveryAction, DeliveryWatchOutcome},
    persistence::Store,
    work_history::WorkActor,
};
use serde_json::{json, Value};

struct Fixture {
    store: Store,
    project: String,
    actor: WorkActor,
}
impl Fixture {
    fn new() -> Self {
        let url =
            std::env::var("NEXUS_TEST_DATABASE_URL").expect("PostgreSQL integration URL required");
        Self {
            store: Store::connect(&url).unwrap(),
            project: format!("delivery-pg-{}", uuid::Uuid::new_v4()),
            actor: WorkActor {
                agent: "pg-agent".into(),
                session_id: "pg-source".into(),
                model_id: None,
                config_hash: "pg-config".into(),
            },
        }
    }
    fn command(&mut self, value: Value) -> anyhow::Result<Value> {
        let action: DeliveryAction = serde_json::from_value(value)?;
        Ok(serde_json::to_value(self.store.delivery_command(
            &self.project,
            &self.actor,
            &action,
        )?)?)
    }
    fn record(&mut self, observations: Vec<Value>) -> anyhow::Result<Value> {
        self.command(json!({"action":"record","observations":observations}))
    }
    fn detail(&mut self, kind: &str, id: &str) -> Value {
        self.command(json!({"action":"get","subject":{"kind":kind,"id":id}}))
            .unwrap()
    }
    fn seed_origins(&mut self) {
        self.record(vec![
            event("c1", contribution("one", "implementer-one")),
            event("c2", contribution("two", "implementer-two")),
            event("m1", mapping("map-one", "one", 10, 'f')),
            event("m2", mapping("map-two", "two", 40, 'e')),
        ])
        .unwrap();
    }
}
fn revision(commit: char) -> Value {
    json!({"repository":"repo","object_format":"sha1","commit":commit.to_string().repeat(40)})
}
fn event(id: &str, body: Value) -> Value {
    json!({"id":id,"occurred_at":"2026-11-15T12:00:00Z","body":body})
}
fn contribution(id: &str, session: &str) -> Value {
    json!({"event":"contribution_registered","contribution":{"id":id,"repository":"repo","title":id,"origin":{"agent":"pg-agent","session_id":session},"features":["routing"],"base_revision":revision('a'),"head_revision":revision('b'),"rationale":"Preserve retry state","constraints":["Keep retries attributable"]}})
}
fn mapping(id: &str, contribution: &str, start: u32, fingerprint: char) -> Value {
    json!({"event":"revision_mapped","mapping":{"id":id,"contribution_id":contribution,"revision":revision('c'),"source_revisions":[revision('b')],"operation":"squash","verification":"mechanically_verified","hunks":[{"path":"src/router.rs","old_start":start,"old_lines":3,"new_start":start,"new_lines":3,"fingerprint":fingerprint.to_string().repeat(64)}]}})
}
fn pull_request(id: &str) -> Value {
    json!({"event":"pull_request_registered","pull_request":{"id":id,"repository":"repo","provider":"github","number":42,"url":"https://github.com/example/repo/pull/42","title":"Release","base":revision('a'),"head":revision('c'),"base_ref":"development","head_ref":"feature","contribution_ids":["one"]}})
}
fn assignment(expected: Option<&str>, session: &str) -> Value {
    json!({"event":"shepherd_assigned","subject":{"kind":"pull_request","id":"pr"},"expected_assignment":expected,"agent":{"agent":"pg-agent","session_id":session},"reason":"handoff"})
}
fn snapshot(head: char, at: &str, target: &str) -> Value {
    json!({"observed_at":at,"state":"open","base":revision('a'),"head":revision(head),"base_ref":target,"head_ref":"feature","checks":[{"id":"checks","name":"tests","state":"succeeded","revision":revision(head)}]})
}

#[test]
fn postgres_origin_indexes_preserve_hunk_attribution_replay_and_project_isolation() {
    let mut source = Fixture::new();
    source.seed_origins();
    let replay = source
        .record(vec![event("m1", mapping("map-one", "one", 10, 'f'))])
        .unwrap();
    assert_eq!(replay["replayed"], 1);
    let query = json!({"action":"resolve_origin","query":{"revision":revision('c'),"path":"src/router.rs","line_start":11,"line_end":12}});
    let found = source.command(query.clone()).unwrap();
    assert_eq!(found["qualification"], "exact");
    assert_eq!(found["matches"].as_array().unwrap().len(), 1);
    assert_eq!(
        found["matches"][0]["contribution"]["origin"]["session_id"],
        "implementer-one"
    );
    let fingerprint=source.command(json!({"action":"resolve_origin","query":{"repository":"repo","fingerprint":"f".repeat(64)}})).unwrap();
    assert_eq!(fingerprint["matches"][0]["contribution"]["id"], "one");
    let mut other = Fixture::new();
    assert_eq!(other.command(query).unwrap()["qualification"], "unknown");
    let features = source
        .command(json!({"action":"list","query":{"kind":"contribution","feature":"routing"}}))
        .unwrap();
    assert_eq!(features["items"].as_array().unwrap().len(), 2);
}

#[test]
fn postgres_pr_identity_and_two_connection_shepherd_handoff_are_atomic() {
    let mut fixture = Fixture::new();
    fixture.seed_origins();
    fixture
        .record(vec![event("pr", pull_request("pr"))])
        .unwrap();
    assert!(fixture
        .record(vec![event("alias", pull_request("duplicate-id"))])
        .is_err());
    fixture
        .record(vec![event("owner-one", assignment(None, "one"))])
        .unwrap();
    let mut contender = Fixture::new();
    contender.project = fixture.project.clone();
    assert!(contender
        .record(vec![event("owner-two", assignment(None, "two"))])
        .is_err());
    contender
        .record(vec![event(
            "owner-two",
            assignment(Some("owner-one"), "two"),
        )])
        .unwrap();
    let found = fixture.detail("pull_request", "pr");
    assert_eq!(found["record"]["shepherd"]["agent"]["session_id"], "two");
    assert_eq!(found["observations"].as_array().unwrap().len(), 3);
    assert_eq!(found["related"].as_array().unwrap().len(), 1);
    let scoped=fixture.command(json!({"action":"resolve_origin","query":{"revision":revision('c'),"pull_request_id":"pr"}})).unwrap();
    assert_eq!(scoped["matches"].as_array().unwrap().len(), 1);
    assert_eq!(scoped["matches"][0]["contribution"]["id"], "one");
}

#[test]
fn postgres_watch_refresh_updates_pinned_snapshot_and_current_target_indexes() {
    let mut fixture = Fixture::new();
    fixture.record(vec![event("pr",pull_request("pr")),event("watch",json!({"event":"watch_registered","watch":{"id":"watch","pull_request_id":"pr","repository_id":"repo","source":{"owner":"example","repository":"repo","number":42},"interval_seconds":30}}))]).unwrap();
    let watch = fixture
        .store
        .claim_delivery_watches(chrono::Utc::now() + chrono::Duration::seconds(1))
        .unwrap()
        .into_iter()
        .find(|watch| watch.project_id == fixture.project)
        .unwrap();
    let observed =
        serde_json::from_value(snapshot('d', "2026-11-15T12:01:00Z", "staging")).unwrap();
    fixture
        .store
        .apply_delivery_watch_poll(
            &watch,
            DeliveryWatchOutcome::Observed {
                snapshot: Box::new(observed),
                provider_updated_at: "2026-11-15T12:01:00Z".parse().unwrap(),
                digest: "f".repeat(64),
            },
        )
        .unwrap();
    let health = fixture.command(json!({"action":"watches"})).unwrap();
    assert!(health["watches"][0]["last_success_at"].is_string());
    let found = fixture.detail("pull_request", "pr");
    assert_eq!(
        found["record"]["body"]["snapshot"]["head"]["commit"],
        "d".repeat(40)
    );
    fixture.record(vec![event("older",json!({"event":"pull_request_observed","pull_request_id":"pr","snapshot":snapshot('c',"2026-11-15T12:00:00Z","development")}))]).unwrap();
    let staging = fixture
        .command(json!({"action":"list","query":{"kind":"pull_request","target":"staging"}}))
        .unwrap();
    assert_eq!(staging["items"].as_array().unwrap().len(), 1);
    let old = fixture
        .command(json!({"action":"list","query":{"kind":"pull_request","target":"development"}}))
        .unwrap();
    assert!(old["items"].as_array().unwrap().is_empty());
    fixture
        .record(vec![event(
            "stop",
            json!({"event":"watch_stopped","watch_id":"watch"}),
        )])
        .unwrap();
}

#[test]
fn postgres_deployment_repair_and_import_preserve_pinned_artifact_and_original_source() {
    let mut fixture = Fixture::new();
    fixture.record(vec![event("c1",contribution("one","implementer-one")),event("rollout",json!({"event":"rollout_registered","rollout":{"id":"release","repository":"repo","title":"Release","contribution_ids":["one"],"environments":["production"]}})),event("deploy",json!({"event":"deployment_recorded","deployment":{"id":"attempt","repository":"repo","rollout_id":"release","environment":"production","revision":revision('c'),"artifact_digest":"sha256:pinned","attempt":1,"outcome":"running"}})),event("finish",json!({"event":"deployment_finished","deployment_id":"attempt","expected_outcome":"running","outcome":"failed"}))]).unwrap();
    assert!(fixture.record(vec![event("stale-finish",json!({"event":"deployment_finished","deployment_id":"attempt","expected_outcome":"running","outcome":"succeeded"}))]).is_err());
    assert_eq!(
        fixture.detail("deployment", "attempt")["record"]["body"]["deployment"]["artifact_digest"],
        "sha256:pinned"
    );
    fixture.record(vec![event("request",json!({"event":"remediation_requested","remediation":{"id":"repair","repository":"repo","title":"Restore retries","recipient":{"agent":"pg-agent","session_id":"implementer-one"},"contribution_id":"one","rollout_id":"release","deployment_id":"attempt","evidence":["https://example.test/failure"],"request":"Restore retry state"}}))]).unwrap();
    let inbox=fixture.command(json!({"action":"inbox","recipient":{"agent":"pg-agent","session_id":"implementer-one"}})).unwrap();
    assert_eq!(inbox["items"].as_array().unwrap().len(), 1);
    fixture.record(vec![event("started",json!({"event":"remediation_transitioned","remediation_id":"repair","expected_state":"requested","state":"in_progress","reason":"reproduced"})),event("resolved",json!({"event":"remediation_transitioned","remediation_id":"repair","expected_state":"in_progress","state":"resolved","reason":"fixed","fixing_revision":revision('d')}))]).unwrap();
    verify_imported_repair(&mut fixture);
}

fn verify_imported_repair(source: &mut Fixture) {
    let exported = source.command(json!({"action":"export"})).unwrap();
    let mut target = Fixture::new();
    target.actor.session_id = "pg-importer".into();
    target
        .command(json!({"action":"import","observations":exported["observations"]}))
        .unwrap();
    let imported = target.detail("remediation", "repair");
    assert_eq!(imported["record"]["state"], "resolved");
    assert_eq!(
        imported["observations"][0]["source"]["project_id"],
        source.project
    );
    assert_eq!(
        imported["observations"][0]["source"]["provenance"]["session_id"],
        "pg-source"
    );
    let inbox=target.command(json!({"action":"inbox","recipient":{"agent":"pg-agent","session_id":"implementer-one"}})).unwrap();
    assert!(inbox["items"].as_array().unwrap().is_empty());
}
