// @feature persistence
// @feature code-concerns
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/code-concerns.md
// @spec docs/features/work-history.md
// @boundary historical-work-fixture: controlled typed observation and projection rows represent an existing V13 database.
use super::*;
use crate::persistence::schema::{
    code_concerns, concern_areas, concern_attributions, concern_decisions, concern_encounters,
    concern_observations, work_links, work_observations, work_subjects,
};
use crate::work_history::{WorkActor, WorkObservationInput};

const PROJECT: &str = "project-before-upgrade";
const TIME: &str = "2026-10-24T12:00:00Z";

#[test]
fn upgrades_v13_preserving_work_history_and_initializing_empty_concern_indexes() {
    let (_temp, path) = database_through("upgrade-v13.db", 13);
    let mut connection = establish_connection(path.to_str().unwrap()).unwrap();
    let payload = seed_work_observation(&mut connection);
    seed_work_projection(&mut connection);
    drop(connection);

    assert_current_version(&path);
    assert_current_version(&path);
    let mut connection = establish_connection(path.to_str().unwrap()).unwrap();
    assert_eq!(
        work_observations::table
            .find((PROJECT, "existing-declaration"))
            .select(work_observations::payload_json)
            .first::<String>(&mut connection)
            .unwrap(),
        payload
    );
    assert_eq!(
        work_subjects::table
            .find((PROJECT, "activity", "existing-tests"))
            .select(work_subjects::title)
            .first::<String>(&mut connection)
            .unwrap(),
        "Existing tests"
    );
    assert_eq!(
        work_links::table
            .find((
                PROJECT,
                "existing-declaration",
                "member_of",
                "session",
                "existing-root"
            ))
            .select(work_links::target_json)
            .first::<String>(&mut connection)
            .unwrap(),
        r#"{"kind":"session","id":"existing-root"}"#
    );
    assert_empty_concern_tables(&mut connection);
}

fn seed_work_observation(connection: &mut DbConnection) -> String {
    let input: WorkObservationInput = serde_json::from_value(serde_json::json!({
        "id":"existing-declaration","subject":{"scope":"activity","id":"existing-tests"},
        "occurred_at":TIME,"producer":{"name":"test-hook","version":"1","instance_id":"existing-process"},
        "method":"caller","body":{"event":"activity_declared","name":"Existing tests","kind":"test.command","work_session_id":"existing-root","executor":"existing-worker","context":{"repository":"repo"}}
    })).unwrap();
    input.validate().unwrap();
    let payload = serde_json::to_string(&input).unwrap();
    let actor = WorkActor {
        agent: "codex".into(),
        session_id: "existing-host".into(),
        model_id: None,
        config_hash: "existing-config".into(),
    };
    diesel::insert_into(work_observations::table)
        .values((
            work_observations::project_id.eq(PROJECT),
            work_observations::id.eq(&input.id),
            work_observations::subject_scope.eq(input.subject.scope()),
            work_observations::subject_id.eq(input.subject.id()),
            work_observations::kind.eq(input.body.family()),
            work_observations::occurred_at.eq(TIME),
            work_observations::recorded_at.eq(TIME),
            work_observations::content_hash
                .eq(blake3::hash(payload.as_bytes()).to_hex().to_string()),
            work_observations::payload_json.eq(&payload),
            work_observations::provenance_json.eq(serde_json::to_string(&actor).unwrap()),
            work_observations::source_json.eq(None::<&str>),
        ))
        .execute(connection)
        .unwrap();
    payload
}

fn seed_work_projection(connection: &mut DbConnection) {
    diesel::insert_into(work_subjects::table)
        .values((
            work_subjects::project_id.eq(PROJECT),
            work_subjects::scope.eq("activity"),
            work_subjects::id.eq("existing-tests"),
            work_subjects::title.eq("Existing tests"),
            work_subjects::kind.eq("test.command"),
            work_subjects::repository.eq(Some("repo")),
            work_subjects::occurred_at.eq(TIME),
            work_subjects::recorded_at.eq(TIME),
        ))
        .execute(connection)
        .unwrap();
    diesel::insert_into(work_links::table)
        .values((
            work_links::project_id.eq(PROJECT),
            work_links::event_id.eq("existing-declaration"),
            work_links::source_scope.eq("activity"),
            work_links::source_id.eq("existing-tests"),
            work_links::relation.eq("member_of"),
            work_links::target_scope.eq("session"),
            work_links::target_id.eq("existing-root"),
            work_links::target_json.eq(r#"{"kind":"session","id":"existing-root"}"#),
        ))
        .execute(connection)
        .unwrap();
}

fn assert_empty_concern_tables(connection: &mut DbConnection) {
    let counts = [
        concern_observations::table
            .count()
            .get_result::<i64>(connection)
            .unwrap(),
        code_concerns::table
            .count()
            .get_result::<i64>(connection)
            .unwrap(),
        concern_encounters::table
            .count()
            .get_result::<i64>(connection)
            .unwrap(),
        concern_areas::table
            .count()
            .get_result::<i64>(connection)
            .unwrap(),
        concern_attributions::table
            .count()
            .get_result::<i64>(connection)
            .unwrap(),
        concern_decisions::table
            .count()
            .get_result::<i64>(connection)
            .unwrap(),
    ];
    assert_eq!(counts, [0; 6]);
}
