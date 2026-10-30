// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// @boundary migration-source-fixture: legacy typed observations verify derived origin backfill and immutable source preservation.
use super::*;
use crate::persistence::schema::{work_links, work_observations, work_subjects};

#[test]
fn upgrades_v12_with_empty_work_observations_and_hierarchy_indexes() {
    let (_temp, path) = database_through("upgrade-v12.db", 12);
    assert_current_version(&path);
    let mut connection = establish_connection(path.to_str().unwrap()).unwrap();
    let counts = [
        work_observations::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        work_subjects::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        work_links::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
    ];
    assert_eq!(counts, [0; 3]);
    let history = flyway_migrations::table
        .order(flyway_migrations::version.asc())
        .select((flyway_migrations::version, flyway_migrations::status))
        .load::<(i64, String)>(&mut connection)
        .unwrap();
    assert_eq!(
        history,
        (0..=CURRENT_SCHEMA_VERSION as i64)
            .map(|version| (version, "deployed".into()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn upgrades_v15_indexing_host_capture_without_changing_original_observations() {
    let (_temp, path) = database_through("upgrade-work-origin.db", 15);
    let mut connection = establish_connection(path.to_str().unwrap()).unwrap();
    let expected = [
        seed_legacy_root(&mut connection, "caller", LegacyRoot::Caller),
        seed_legacy_root(&mut connection, "host", LegacyRoot::TaggedHost),
    ];
    drop(connection);
    assert_current_version(&path);
    assert_current_version(&path);
    let mut connection = establish_connection(path.to_str().unwrap()).unwrap();
    let flags = work_subjects::table
        .order(work_subjects::id.asc())
        .select((work_subjects::id, work_subjects::has_host_capture))
        .load::<(String, bool)>(&mut connection)
        .unwrap();
    assert_eq!(flags, vec![("caller".into(), false), ("host".into(), true)]);
    let payloads = work_observations::table
        .order(work_observations::id.asc())
        .select(work_observations::payload_json)
        .load::<String>(&mut connection)
        .unwrap();
    assert_eq!(payloads, expected);
}

#[derive(Clone, Copy)]
enum LegacyRoot {
    Caller,
    TaggedHost,
    Companion,
}

fn seed_legacy_root(connection: &mut DbConnection, id: &str, origin: LegacyRoot) -> String {
    use crate::work_history::{WorkActor, WorkObservationInput};
    let labels = if matches!(origin, LegacyRoot::TaggedHost) {
        serde_json::json!({"nexus.capture.host":"fixture"})
    } else {
        serde_json::json!({})
    };
    let input: WorkObservationInput = serde_json::from_value(serde_json::json!({
        "id": id, "subject":{"scope":"session","id":id}, "occurred_at":"2026-10-24T12:00:00Z",
        "producer":{"name":if matches!(origin, LegacyRoot::Companion) { "nexus-companion" } else { "fixture" },"version":"1","instance_id":"clock"},"method":"caller",
        "body":{"event":"session_opened","title":id,"context":{"labels":labels,"host_session_id":"legacy-native"}}
    }))
    .unwrap();
    input.validate().unwrap();
    let payload = serde_json::to_string(&input).unwrap();
    let actor = WorkActor {
        agent: if matches!(origin, LegacyRoot::Companion) {
            "codex"
        } else {
            "fixture"
        }
        .into(),
        session_id: "legacy-native".into(),
        model_id: None,
        config_hash: "fixture".into(),
    };
    diesel::insert_into(work_observations::table)
        .values((
            work_observations::project_id.eq("project"),
            work_observations::id.eq(id),
            work_observations::subject_scope.eq("session"),
            work_observations::subject_id.eq(id),
            work_observations::kind.eq("session_opened"),
            work_observations::occurred_at.eq("2026-10-24T12:00:00Z"),
            work_observations::recorded_at.eq("2026-10-24T12:00:00Z"),
            work_observations::payload_json.eq(&payload),
            work_observations::content_hash
                .eq(blake3::hash(payload.as_bytes()).to_hex().to_string()),
            work_observations::provenance_json.eq(serde_json::to_string(&actor).unwrap()),
            work_observations::source_json.eq(None::<&str>),
        ))
        .execute(connection)
        .unwrap();
    seed_legacy_subject(connection, id);
    payload
}

fn seed_legacy_subject(connection: &mut DbConnection, id: &str) {
    diesel::insert_into(work_subjects::table)
        .values((
            work_subjects::project_id.eq("project"),
            work_subjects::scope.eq("session"),
            work_subjects::id.eq(id),
            work_subjects::title.eq(id),
            work_subjects::kind.eq("work.session"),
            work_subjects::occurred_at.eq("2026-10-24T12:00:00Z"),
            work_subjects::recorded_at.eq("2026-10-24T12:00:00Z"),
        ))
        .execute(connection)
        .unwrap();
}

#[test]
fn upgrades_v15_backfilling_a_companion_root_without_capture_labels() {
    let (_temp, path) = database_through("legacy-companion.db", 15);
    let mut connection = establish_connection(path.to_str().unwrap()).unwrap();
    let id = "58448194a44a55a3e6f7e0e780fdcfe223da3649b0c53a4ca1e0b85eadd89329";
    let payload = seed_legacy_root(&mut connection, id, LegacyRoot::Companion);
    drop(connection);
    assert_current_version(&path);
    let mut connection = establish_connection(path.to_str().unwrap()).unwrap();
    assert!(work_subjects::table
        .find(("project", "session", id))
        .select(work_subjects::has_host_capture)
        .first::<bool>(&mut connection)
        .unwrap());
    assert_eq!(
        work_observations::table
            .find(("project", id))
            .select(work_observations::payload_json)
            .first::<String>(&mut connection)
            .unwrap(),
        payload
    );
}
