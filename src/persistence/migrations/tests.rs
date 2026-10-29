// @feature persistence
// @spec docs/features/persistence.md
use super::*;
use flyway::ChangelogFile;

const CURRENT_SCHEMA_VERSION: u64 = 15;

#[path = "work_history_tests.rs"]
mod work_history;

#[path = "code_concerns_tests.rs"]
mod code_concerns;

struct ThroughVersion(u64);

struct OneMigration(ChangelogFile);

impl MigrationStore for ThroughVersion {
    fn changelogs(&self) -> Vec<ChangelogFile> {
        NexusMigrations {}
            .changelogs()
            .into_iter()
            .filter(|changelog| changelog.version() <= self.0)
            .collect()
    }
}

impl MigrationStore for OneMigration {
    fn changelogs(&self) -> Vec<ChangelogFile> {
        vec![self.0.clone()]
    }
}

fn database_through(filename: &str, version: u64) -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(filename);
    let deployed = migrate_with(&path, ThroughVersion(version)).unwrap();
    assert_eq!(deployed, Some(version));
    (temp, path)
}

fn assert_current_version(path: &Path) {
    let deployed = migrate_database(path).unwrap();
    assert_eq!(deployed, Some(CURRENT_SCHEMA_VERSION));
}

#[test]
fn failed_migration_rolls_back_schema_and_history_so_the_version_can_retry() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("failed-migration.db");
    let failed = ChangelogFile::from_string(
        91,
        "atomic_failure",
        "CREATE TABLE migration_atomicity_probe (id INTEGER PRIMARY KEY);\n\
         INSERT INTO migration_atomicity_missing (id) VALUES (1);",
    )
    .unwrap();

    assert!(migrate_with(&path, OneMigration(failed)).is_err());
    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    assert_eq!(
        flyway_migrations::table
            .filter(flyway_migrations::version.eq(91))
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        0,
        "failed migration history must roll back with its schema changes"
    );
    drop(connection);

    let retry = ChangelogFile::from_string(
        91,
        "atomic_retry",
        "CREATE TABLE migration_atomicity_probe (id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    assert_eq!(migrate_with(&path, OneMigration(retry)).unwrap(), Some(91));

    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    let statuses = flyway_migrations::table
        .filter(flyway_migrations::version.eq(91))
        .select(flyway_migrations::status)
        .load::<String>(&mut connection)
        .unwrap();
    assert_eq!(statuses, vec!["deployed"]);
}

#[test]
fn applies_only_unseen_versions_and_is_idempotent() {
    let (_temp, path) = database_through("upgrade.db", 0);
    assert_current_version(&path);
    assert_current_version(&path);
}

#[test]
fn portable_export_validation_accepts_a_supported_database_after_upgrade() {
    let (_temp, path) = database_through("portable-upgrade.db", 5);
    assert_current_version(&path);
    let mut connection = establish_connection(path.to_str().unwrap()).unwrap();

    validate_portable_source_migrations(&mut connection).unwrap();
}

#[test]
fn upgrades_an_existing_v2_database_without_losing_events() {
    use super::super::models::NewEvent;
    use super::super::schema::events;

    let (_temp, path) = database_through("upgrade-v2.db", 2);

    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    diesel::insert_into(events::table)
        .values(NewEvent {
            project_id: "project-before-upgrade",
            session_id: None,
            kind: "pre_upgrade_event",
            payload_json: "{}".to_owned(),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .execute(&mut connection)
        .unwrap();
    drop(connection);

    assert_current_version(&path);
    let mut store = super::super::Store::open(&path).unwrap();
    let events = store
        .list_events(Some("project-before-upgrade"), 10)
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "pre_upgrade_event");
    assert!(store
        .usage_summary(
            None,
            chrono::Utc::now() - chrono::Duration::days(7),
            chrono::Utc::now(),
        )
        .unwrap()
        .tools
        .is_empty());
}

#[test]
fn upgrades_an_existing_v3_database_with_agent_memory_tables() {
    use super::super::schema::memory_spaces;

    let (_temp, path) = database_through("upgrade-v3.db", 3);
    assert_current_version(&path);

    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    assert_eq!(
        memory_spaces::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        0
    );
}

#[test]
fn upgrades_an_existing_v4_database_with_empty_papercut_registry() {
    use super::super::schema::{
        papercut_duplicate_reviews, papercut_review_attempts, papercut_upvotes, papercuts,
    };

    let (_temp, path) = database_through("upgrade-v4.db", 4);
    assert_current_version(&path);

    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    assert_eq!(
        papercuts::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        0
    );
    assert_eq!(
        papercut_upvotes::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        0
    );
    assert_eq!(
        papercut_duplicate_reviews::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        0
    );
    assert_eq!(
        papercut_review_attempts::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        0
    );
}

#[test]
fn upgrades_v5_sessions_to_project_scoped_identity_without_losing_rows() {
    use super::super::models::NewSession;
    use super::super::schema::sessions;
    use crate::coordination::domain::RecordScope;

    let (_temp, path) = database_through("upgrade-v5.db", 5);

    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    diesel::insert_into(sessions::table)
        .values(NewSession {
            session_id: "shared-session",
            project_id: "project-before-upgrade",
            agent: "codex",
            worktree: None,
            task_summary: None,
            prompt_hash: None,
            status: "active",
            config_hash: "config",
            started_at: "2026-01-01T00:00:00Z",
            last_seen_at: "2026-01-01T00:00:00Z",
        })
        .execute(&mut connection)
        .unwrap();
    drop(connection);

    assert_current_version(&path);
    let mut store = super::super::Store::open(&path).unwrap();
    store
        .touch_session(
            "project-after-upgrade",
            "shared-session",
            "claude",
            None,
            "config",
        )
        .unwrap();
    let sessions = store.list_sessions(RecordScope::All, 120).unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(sessions
        .iter()
        .any(|session| session.project_id == "project-before-upgrade"));
    assert!(sessions
        .iter()
        .any(|session| session.project_id == "project-after-upgrade"));
}

#[test]
fn upgrades_v6_event_history_to_enforced_storage_bounds() {
    use super::super::models::NewEvent;
    use super::super::schema::events;

    let (_temp, path) = database_through("upgrade-v6.db", 6);

    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    diesel::insert_into(events::table)
        .values(NewEvent {
            project_id: "project",
            session_id: Some("session"),
            kind: "legacy_oversized",
            payload_json: "x".repeat(65_537),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .execute(&mut connection)
        .unwrap();
    drop(connection);

    assert_current_version(&path);
    let mut store = super::super::Store::open(&path).unwrap();
    let stored = store.list_events(Some("project"), 1).unwrap();
    assert_eq!(
        serde_json::to_value(&stored[0].payload).unwrap()["reason"],
        "oversized_historical_event_payload"
    );

    let oversized_insert = diesel::insert_into(events::table)
        .values(NewEvent {
            project_id: "project",
            session_id: Some("session"),
            kind: "new_oversized",
            payload_json: "x".repeat(65_537),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .execute(&mut store.connection);
    assert!(oversized_insert.is_err());
}

struct LegacyProjectionFixture {
    legacy_session_id: String,
    legacy_claim_id: String,
    legacy_tool_use_id: String,
    legacy_agent: String,
    legacy_worktree: String,
    legacy_summary: String,
    legacy_path: String,
    legacy_message: String,
    colliding_session_id: &'static str,
    colliding_claim_id: &'static str,
    colliding_tool_use_id: &'static str,
    colliding_path: String,
    now: &'static str,
}

impl LegacyProjectionFixture {
    fn new() -> Self {
        Self {
            legacy_session_id: format!("legacy-session-{}", "é".repeat(260)),
            legacy_claim_id: format!("legacy-claim-{}", "é".repeat(260)),
            legacy_tool_use_id: format!("legacy-tool-{}", "é".repeat(260)),
            legacy_agent: "é".repeat(129),
            legacy_worktree: format!("/{}", "é".repeat(2_050)),
            legacy_summary: "é".repeat(513),
            legacy_path: format!("src/{}", "é".repeat(2_050)),
            legacy_message: "é".repeat(4_097),
            colliding_session_id: "nexus-v8-session-0000000000000001",
            colliding_claim_id: "nexus-v8-claim-0000000000000001",
            colliding_tool_use_id: "nexus-v8-tool-0000000000000001",
            colliding_path: format!("src/{}~nexus-v8-0000000000000001", "é".repeat(2_033)),
            now: "2026-01-01T00:00:00Z",
        }
    }
}

fn insert_collision_projection(connection: &mut DbConnection, fixture: &LegacyProjectionFixture) {
    use super::super::models::{NewClaim, NewSession};
    use super::super::schema::{claims, sessions};

    diesel::insert_into(sessions::table)
        .values(NewSession {
            session_id: fixture.colliding_session_id,
            project_id: "project",
            agent: "collision-agent",
            worktree: None,
            task_summary: None,
            prompt_hash: None,
            status: "active",
            config_hash: "collision-config",
            started_at: fixture.now,
            last_seen_at: fixture.now,
        })
        .execute(connection)
        .unwrap();
    diesel::insert_into(claims::table)
        .values(NewClaim {
            id: fixture.colliding_claim_id,
            project_id: "project",
            session_id: fixture.colliding_session_id,
            tool_use_id: fixture.colliding_tool_use_id,
            path: &fixture.colliding_path,
            operation: "read",
            line_start: -1,
            line_end: -1,
            state: "claimed",
            expires_at: fixture.now,
            updated_at: fixture.now,
        })
        .execute(connection)
        .unwrap();
}

fn insert_legacy_projection(connection: &mut DbConnection, fixture: &LegacyProjectionFixture) {
    use super::super::models::{NewClaim, NewConflict, NewSession};
    use super::super::schema::{claims, conflicts, sessions};

    diesel::insert_into(sessions::table)
        .values(NewSession {
            session_id: &fixture.legacy_session_id,
            project_id: "project",
            agent: &fixture.legacy_agent,
            worktree: Some(&fixture.legacy_worktree),
            task_summary: Some(&fixture.legacy_summary),
            prompt_hash: None,
            status: "active",
            config_hash: "legacy-config",
            started_at: fixture.now,
            last_seen_at: fixture.now,
        })
        .execute(connection)
        .unwrap();
    diesel::insert_into(claims::table)
        .values(NewClaim {
            id: &fixture.legacy_claim_id,
            project_id: "project",
            session_id: &fixture.legacy_session_id,
            tool_use_id: &fixture.legacy_tool_use_id,
            path: &fixture.legacy_path,
            operation: "write",
            line_start: -1,
            line_end: -1,
            state: "claimed",
            expires_at: fixture.now,
            updated_at: fixture.now,
        })
        .execute(connection)
        .unwrap();
    diesel::insert_into(conflicts::table)
        .values(NewConflict {
            id: "conflict",
            project_id: "project",
            left_claim_id: &fixture.legacy_claim_id,
            right_claim_id: &fixture.legacy_claim_id,
            path: &fixture.legacy_path,
            severity: "warning",
            kind: "path_overlap",
            status: "open",
            message: &fixture.legacy_message,
            created_at: fixture.now,
            updated_at: fixture.now,
        })
        .execute(connection)
        .unwrap();
}

fn assert_migrated_session(
    connection: &mut DbConnection,
    fixture: &LegacyProjectionFixture,
) -> String {
    use super::super::schema::sessions;

    let (session_id, agent, worktree, summary) = sessions::table
        .filter(sessions::project_id.eq("project"))
        .filter(sessions::config_hash.eq("legacy-config"))
        .select((
            sessions::session_id,
            sessions::agent,
            sessions::worktree,
            sessions::task_summary,
        ))
        .first::<(String, String, Option<String>, Option<String>)>(connection)
        .unwrap();
    assert!(session_id.len() <= 512);
    assert_ne!(session_id, fixture.colliding_session_id);
    assert!(agent.len() <= 256);
    assert!(worktree.unwrap().len() <= 4_096);
    assert!(summary.unwrap().len() <= 1_024);
    session_id
}

fn assert_migrated_claim(
    connection: &mut DbConnection,
    fixture: &LegacyProjectionFixture,
    session_id: &str,
) -> (String, String) {
    use super::super::schema::claims;

    let (claim_id, claim_session_id, tool_use_id, claim_path) = claims::table
        .filter(claims::operation.eq("write"))
        .select((
            claims::id,
            claims::session_id,
            claims::tool_use_id,
            claims::path,
        ))
        .first::<(String, String, String, String)>(connection)
        .unwrap();
    assert!(claim_id.len() <= 512);
    assert_ne!(claim_id, fixture.colliding_claim_id);
    assert_eq!(claim_session_id, session_id);
    assert!(tool_use_id.len() <= 512);
    assert_ne!(tool_use_id, fixture.colliding_tool_use_id);
    assert!(claim_path.len() <= 4_096);
    assert_ne!(claim_path, fixture.colliding_path);
    (claim_id, claim_path)
}

fn assert_migrated_conflict(connection: &mut DbConnection, claim_id: &str, claim_path: &str) {
    use super::super::schema::conflicts;

    let (left_claim_id, right_claim_id, conflict_path, message) = conflicts::table
        .select((
            conflicts::left_claim_id,
            conflicts::right_claim_id,
            conflicts::path,
            conflicts::message,
        ))
        .first::<(String, String, String, String)>(connection)
        .unwrap();
    assert_eq!(left_claim_id, claim_id);
    assert_eq!(right_claim_id, claim_id);
    assert!(conflict_path.len() <= 4_096);
    assert_eq!(conflict_path, claim_path);
    assert!(message.len() <= 8_192);
}

fn assert_session_bound_enforced(connection: &mut DbConnection, fixture: &LegacyProjectionFixture) {
    use super::super::models::NewSession;
    use super::super::schema::sessions;

    let oversized_insert = diesel::insert_into(sessions::table)
        .values(NewSession {
            session_id: "new-session",
            project_id: "project",
            agent: &"x".repeat(257),
            worktree: None,
            task_summary: None,
            prompt_hash: None,
            status: "active",
            config_hash: "config",
            started_at: fixture.now,
            last_seen_at: fixture.now,
        })
        .execute(connection);
    assert!(oversized_insert.is_err());
}

fn assert_claim_bound_enforced(connection: &mut DbConnection, session_id: &str, now: &str) {
    use super::super::models::NewClaim;
    use super::super::schema::claims;

    let oversized_insert = diesel::insert_into(claims::table)
        .values(NewClaim {
            id: "new-claim",
            project_id: "project",
            session_id,
            tool_use_id: "new-tool",
            path: &"x".repeat(4_097),
            operation: "write",
            line_start: -1,
            line_end: -1,
            state: "claimed",
            expires_at: now,
            updated_at: now,
        })
        .execute(connection);
    assert!(oversized_insert.is_err());
}

fn assert_conflict_bound_enforced(connection: &mut DbConnection) {
    use super::super::schema::conflicts;

    let oversized_update = diesel::update(conflicts::table)
        .set(conflicts::message.eq("x".repeat(8_193)))
        .execute(connection);
    assert!(oversized_update.is_err());
}

#[test]
fn upgrades_v7_coordination_projections_to_enforced_storage_bounds() {
    let (_temp, path) = database_through("upgrade-v7.db", 7);
    let fixture = LegacyProjectionFixture::new();
    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    insert_collision_projection(&mut connection, &fixture);
    insert_legacy_projection(&mut connection, &fixture);
    drop(connection);

    assert_current_version(&path);
    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    let session_id = assert_migrated_session(&mut connection, &fixture);
    let (claim_id, claim_path) = assert_migrated_claim(&mut connection, &fixture, &session_id);
    assert_migrated_conflict(&mut connection, &claim_id, &claim_path);
    assert_session_bound_enforced(&mut connection, &fixture);
    assert_claim_bound_enforced(&mut connection, &session_id, fixture.now);
    assert_conflict_bound_enforced(&mut connection);
}

#[test]
fn upgrades_v10_with_empty_activity_projections() {
    use super::super::schema::{
        analytics_actions, analytics_calls, analytics_categories, analytics_exposures,
        analytics_resources, analytics_revisions,
    };
    let (_temp, path) = database_through("upgrade-v10.db", 10);
    assert_current_version(&path);
    let mut connection = establish_connection(path.to_str().unwrap()).unwrap();
    let counts = [
        analytics_calls::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        analytics_actions::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        analytics_exposures::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        analytics_resources::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        analytics_categories::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        analytics_revisions::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
    ];
    assert_eq!(counts, [0; 6]);
}
