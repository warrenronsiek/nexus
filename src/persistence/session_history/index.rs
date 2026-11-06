// @feature persistence
// @feature coordination
// @spec docs/features/persistence.md
// @spec docs/features/coordination.md
//! The project session picker, ranked by recent coordination activity.
use super::{display_name, with_owners};
use crate::coordination::domain::{query_window_limit, RecordWindow, SessionActivity};
use crate::persistence::backend::DbConnection;
use crate::persistence::models::{parse_time, session_from_row, ConflictRow, SessionRow};
use crate::persistence::schema::{advisories, claims, conflicts, sessions};
use crate::persistence::Store;
use anyhow::Result;
use chrono::{DateTime, Utc};
use diesel::dsl::{count, count_star, max};
use diesel::prelude::*;
use std::collections::{BTreeSet, HashMap};

impl Store {
    /// Ranks sessions by their latest coordination activity: the later of their last
    /// observation and the latest update of a conflict they are party to.
    pub(crate) fn session_index(
        &mut self,
        project_key: &str,
        limit: usize,
    ) -> Result<RecordWindow<SessionActivity>> {
        let connection = &mut self.connection;
        let candidates = index_candidates(connection, project_key, limit)?;
        let ids = candidates
            .iter()
            .map(|row| row.session_id.clone())
            .collect::<Vec<_>>();
        let stats = SessionStats::load(connection, project_key, &ids)?;
        let mut ranked = stats.rank(candidates, limit)?;
        for activity in &mut ranked {
            activity.session.display_name = display_name(connection, &activity.session)?;
        }
        Ok(RecordWindow {
            items: ranked,
            truncated: has_more_sessions(connection, project_key, limit)?,
        })
    }
}

/// The most recently seen sessions plus quiet owners of the most recently updated
/// conflicts, whose overlaps may change after they were last seen.
fn index_candidates(
    connection: &mut DbConnection,
    project_key: &str,
    limit: usize,
) -> Result<Vec<SessionRow>> {
    let window = query_window_limit(limit);
    let mut candidates = sessions::table
        .filter(sessions::project_id.eq(project_key))
        .order((sessions::last_seen_at.desc(), sessions::session_id.asc()))
        .limit(window)
        .select(SessionRow::as_select())
        .load::<SessionRow>(connection)?;
    let seen = candidates
        .iter()
        .map(|row| row.session_id.clone())
        .collect::<BTreeSet<_>>();
    let quiet = recent_conflict_owners(connection, project_key, window)?
        .into_iter()
        .filter(|session| !seen.contains(session))
        .collect::<BTreeSet<_>>();
    candidates.extend(
        sessions::table
            .filter(sessions::project_id.eq(project_key))
            .filter(sessions::session_id.eq_any(&quiet))
            .select(SessionRow::as_select())
            .load::<SessionRow>(connection)?,
    );
    Ok(candidates)
}

fn recent_conflict_owners(
    connection: &mut DbConnection,
    project_key: &str,
    window: i64,
) -> Result<BTreeSet<String>> {
    let recent = conflicts::table
        .filter(conflicts::project_id.eq(project_key))
        .order((conflicts::updated_at.desc(), conflicts::id.asc()))
        .limit(window)
        .select(ConflictRow::as_select())
        .load::<ConflictRow>(connection)?;
    Ok(with_owners(connection, project_key, recent)?
        .into_iter()
        .flat_map(|conflict| [conflict.left, conflict.right])
        .flatten()
        .collect())
}

fn has_more_sessions(
    connection: &mut DbConnection,
    project_key: &str,
    limit: usize,
) -> Result<bool> {
    let total = sessions::table
        .filter(sessions::project_id.eq(project_key))
        .select(count_star())
        .first::<i64>(connection)?;
    Ok(usize::try_from(total).unwrap_or(usize::MAX) > limit)
}

#[derive(Default)]
struct SessionStats {
    conflicts: HashMap<String, i64>,
    latest_conflict: HashMap<String, DateTime<Utc>>,
    advisories: HashMap<String, i64>,
}

impl SessionStats {
    fn load(connection: &mut DbConnection, project_key: &str, ids: &[String]) -> Result<Self> {
        let mut stats = Self {
            advisories: delivered_advisory_counts(connection, project_key, ids)?,
            ..Self::default()
        };
        for (session, conflicts, latest) in party_conflict_stats(connection, project_key, ids)? {
            if let Some(latest) = latest {
                stats
                    .latest_conflict
                    .insert(session.clone(), parse_time(&latest)?);
            }
            stats.conflicts.insert(session, conflicts);
        }
        Ok(stats)
    }

    fn activity(&self, row: SessionRow) -> Result<SessionActivity> {
        let session = session_from_row(row)?;
        let id = &session.session_id;
        Ok(SessionActivity {
            last_activity_at: self
                .latest_conflict
                .get(id)
                .map_or(session.last_seen_at, |at| (*at).max(session.last_seen_at)),
            conflicts: self.conflicts.get(id).copied().unwrap_or(0),
            advisories: self.advisories.get(id).copied().unwrap_or(0),
            session,
        })
    }

    fn rank(&self, candidates: Vec<SessionRow>, limit: usize) -> Result<Vec<SessionActivity>> {
        let mut ranked = candidates
            .into_iter()
            .map(|row| self.activity(row))
            .collect::<Result<Vec<_>>>()?;
        ranked.sort_by(|left, right| {
            right
                .last_activity_at
                .cmp(&left.last_activity_at)
                .then_with(|| left.session.session_id.cmp(&right.session.session_id))
        });
        ranked.truncate(limit);
        Ok(ranked)
    }
}

/// Per owner session: conflicts in which it owns either claim and their latest update.
/// The two claims of a conflict always belong to different sessions.
fn party_conflict_stats(
    connection: &mut DbConnection,
    project_key: &str,
    ids: &[String],
) -> Result<Vec<(String, i64, Option<String>)>> {
    Ok(conflicts::table
        .inner_join(
            claims::table.on(claims::id
                .eq(conflicts::left_claim_id)
                .or(claims::id.eq(conflicts::right_claim_id))),
        )
        .filter(conflicts::project_id.eq(project_key))
        .filter(claims::project_id.eq(project_key))
        .filter(claims::session_id.eq_any(ids))
        .group_by(claims::session_id)
        .select((
            claims::session_id,
            count(conflicts::id),
            max(conflicts::updated_at),
        ))
        .load(connection)?)
}

fn delivered_advisory_counts(
    connection: &mut DbConnection,
    project_key: &str,
    ids: &[String],
) -> Result<HashMap<String, i64>> {
    Ok(advisories::table
        .filter(advisories::project_id.eq(project_key))
        .filter(advisories::session_id.eq_any(ids))
        .group_by(advisories::session_id)
        .select((advisories::session_id, count_star()))
        .load::<(String, i64)>(connection)?
        .into_iter()
        .collect())
}
