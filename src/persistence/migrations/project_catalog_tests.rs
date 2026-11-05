// @feature persistence
// @feature observability-ui
// @spec docs/features/persistence.md
// @spec docs/features/observability-ui.md
use super::*;

#[test]
fn upgrade_to_v19_catalogs_every_project_with_recorded_history() {
    use crate::persistence::schema::projects;
    let (_temp, path) = database_through("catalog-backfill.db", 18);
    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    for statement in [
        "INSERT INTO sessions (project_id, session_id, agent, status, config_hash, started_at, last_seen_at) VALUES ('hooks', 's', 'codex', 'active', 'c', '2026-11-01T10:00:00+00:00', '2026-11-03T10:00:00+00:00')",
        "INSERT INTO events (project_id, kind, payload_json, created_at) VALUES ('hooks', 'prompt', '{}', '2026-11-04T10:00:00+00:00')",
        "INSERT INTO memory_spaces (id, scope, project_id, created_at, updated_at) VALUES ('global', 'global', NULL, '2026-11-05T10:00:00+00:00', '2026-11-05T10:00:00+00:00')",
        "INSERT INTO memory_spaces (id, scope, project_id, created_at, updated_at) VALUES ('notes', 'project', 'notes', '2026-11-02T10:00:00+00:00', '2026-11-02T11:00:00+00:00')",
    ] {
        diesel::sql_query(statement).execute(&mut connection).unwrap();
    }
    drop(connection);
    assert_current_version(&path);
    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    let catalog = projects::table
        .order(projects::project_id.asc())
        .select((
            projects::project_id,
            projects::first_seen_at,
            projects::last_seen_at,
        ))
        .load::<(String, String, String)>(&mut connection)
        .unwrap();
    assert_eq!(
        catalog,
        [
            (
                "hooks".to_owned(),
                "2026-11-01T10:00:00+00:00".to_owned(),
                "2026-11-04T10:00:00+00:00".to_owned()
            ),
            (
                "notes".to_owned(),
                "2026-11-02T10:00:00+00:00".to_owned(),
                "2026-11-02T11:00:00+00:00".to_owned()
            ),
        ]
    );
}
