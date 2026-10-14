// @feature persistence
// @feature coordination
// @spec docs/features/persistence.md
// @spec docs/features/coordination.md
use super::schema::{claims, sessions};
use super::store::{append_event, Store};
use crate::coordination::domain::ClaimRelease;
use crate::persistence::backend::DbConnection;
use anyhow::Result;
use chrono::Utc;
use diesel::prelude::*;
use serde_json::json;

impl Store {
    pub(crate) fn release_claims(
        &mut self,
        project_key: &str,
        session_key: &str,
        release: &ClaimRelease,
    ) -> Result<usize> {
        self.transaction(|connection| {
            release_claims_transaction(connection, project_key, session_key, release)
        })
    }
}

fn release_claims_transaction(
    connection: &mut DbConnection,
    project_key: &str,
    session_key: &str,
    release: &ClaimRelease,
) -> Result<usize> {
    let session_exists = session_exists(connection, project_key, session_key)?;
    let now = Utc::now().to_rfc3339();
    let count = release_claim_rows(connection, project_key, session_key, release, &now)?;
    if session_exists {
        append_event(
            connection,
            project_key,
            Some(session_key),
            "claims_released",
            json!({"path":released_path(release),"count":count}),
        )?;
    }
    Ok(count)
}

fn session_exists(
    connection: &mut DbConnection,
    project_key: &str,
    session_key: &str,
) -> Result<bool> {
    Ok(sessions::table
        .filter(sessions::project_id.eq(project_key))
        .filter(sessions::session_id.eq(session_key))
        .select(sessions::session_id)
        .first::<String>(connection)
        .optional()?
        .is_some())
}

fn release_claim_rows(
    connection: &mut DbConnection,
    project_key: &str,
    session_key: &str,
    release: &ClaimRelease,
    now: &str,
) -> Result<usize> {
    let mut target = diesel::update(claims::table)
        .filter(claims::project_id.eq(project_key))
        .filter(claims::session_id.eq(session_key))
        .filter(claims::state.eq_any(["claimed", "modified"]))
        .into_boxed();
    if let ClaimRelease::Path(path) = release {
        target = target.filter(claims::path.eq(path));
    }
    Ok(target
        .set((claims::state.eq("released"), claims::updated_at.eq(now)))
        .execute(connection)?)
}

fn released_path(release: &ClaimRelease) -> Option<&str> {
    match release {
        ClaimRelease::Session => None,
        ClaimRelease::Path(path) => Some(path),
    }
}
