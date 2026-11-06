// @feature persistence
// @feature coordination
// @spec docs/features/persistence.md
// @spec docs/features/coordination.md
use super::models::{conflict_from_row, parse_time, ConflictRow, NewAdvisory, NewConflict};
use super::schema::{advisories, claims, conflicts, sessions};
use super::store::{append_event, Store};
use super::window::bounded_projection_reader;
use crate::agents::analyst::AnalysisResult;
use crate::coordination::domain::{Advisory, Claim, ConflictRecord, ConflictScope, ConflictStatus};
use crate::persistence::backend::DbConnection;
use anyhow::Result;
use chrono::{DateTime, Utc};
use diesel::dsl::max;
use diesel::prelude::*;
use serde_json::json;

/// One closure pass is bounded so it never holds a store for an unbounded backlog.
pub(crate) const MAX_CONFLICT_CLOSURES: usize = 64;

struct ConflictObservation<'a> {
    project_key: &'a str,
    left_claim: &'a Claim,
    right_claim: &'a Claim,
    advisory: &'a Advisory,
    tool_key: &'a str,
    conflict_id: &'a str,
    severity: &'a str,
    now: &'a str,
}

impl Store {
    pub(crate) fn record_conflict(
        &mut self,
        project_key: &str,
        left_claim: &Claim,
        right_claim: &Claim,
        mut advisory: Advisory,
        tool_key: &str,
    ) -> Result<Advisory> {
        let now = Utc::now().to_rfc3339();
        let conflict_id = conflict_id(project_key, &advisory.path, left_claim, right_claim)?;
        advisory.conflict_id = Some(conflict_id.clone());
        let severity = advisory.severity.to_string();
        let result = {
            let observation = ConflictObservation {
                project_key,
                left_claim,
                right_claim,
                advisory: &advisory,
                tool_key,
                conflict_id: &conflict_id,
                severity: &severity,
                now: &now,
            };
            self.transaction(|connection| {
                merge_projection(connection, &observation)?;
                record_occurrence(connection, &observation)
            })
        };
        result.map(|()| advisory)
    }

    pub fn list_conflicts(
        &mut self,
        project_key: Option<&str>,
        scope: ConflictScope,
    ) -> Result<Vec<ConflictRecord>> {
        Ok(self.list_conflicts_window(project_key, scope)?.items)
    }

    bounded_projection_reader! {
        pub(crate) fn list_conflicts_window(project_key, scope: ConflictScope) -> ConflictRecord {
            table: conflicts::table,
            project_column: conflicts::project_id,
            scoped_when: scope == ConflictScope::Open,
            scoped_filter: super::liveness::live_conflict(),
            order: (conflicts::created_at.desc(), conflicts::id.asc()),
            row: ConflictRow,
            decode: conflict_from_row,
            maximum: crate::coordination::api::MAX_CONFLICT_RESULTS,
        }
    }

    /// Moves stored-open conflicts whose overlap ended to `ended`, at most one bounded pass.
    pub(crate) fn end_stale_conflicts(&mut self, project_key: Option<&str>) -> Result<usize> {
        self.transaction(|connection| end_stale_conflicts(connection, project_key))
    }

    pub(crate) fn conflict_by_id(&mut self, conflict_key: &str) -> Result<Option<ConflictRecord>> {
        find_conflict(&mut self.connection, conflict_key)
    }

    pub(crate) fn conflict_worktree(
        &mut self,
        conflict: &ConflictRecord,
    ) -> Result<Option<String>> {
        let session_ids = claims::table
            .filter(claims::project_id.eq(&conflict.project_id))
            .filter(claims::id.eq_any([
                conflict.left_claim_id.as_str(),
                conflict.right_claim_id.as_str(),
            ]))
            .select(claims::session_id)
            .load::<String>(&mut self.connection)?;
        for session_id in session_ids {
            let worktree = sessions::table
                .filter(sessions::project_id.eq(&conflict.project_id))
                .filter(sessions::session_id.eq(session_id))
                .select(sessions::worktree)
                .first::<Option<String>>(&mut self.connection)
                .optional()?
                .flatten();
            if worktree.is_some() {
                return Ok(worktree);
            }
        }
        Ok(None)
    }

    pub(crate) fn record_analysis(
        &mut self,
        project_key: &str,
        conflict_key: &str,
        analysis: &AnalysisResult,
        config_hash: &str,
    ) -> Result<()> {
        append_event(
            &mut self.connection,
            project_key,
            None,
            "conflict_analyzed",
            json!({"conflict_id":conflict_key,"analysis":analysis,"config_hash":config_hash}),
        )
    }

    pub(crate) fn resolve_conflict(
        &mut self,
        conflict_key: &str,
        resolution: &str,
    ) -> Result<bool> {
        self.transaction(|connection| {
            let row = conflicts::table
                .filter(conflicts::id.eq(conflict_key))
                .select((conflicts::project_id, conflicts::status))
                .first::<(String, String)>(connection)
                .optional()?;
            let Some((project_key, current_status)) = row else {
                return Ok(false);
            };
            if current_status == "open" {
                diesel::update(conflicts::table.filter(conflicts::id.eq(conflict_key)))
                    .set((
                        conflicts::status.eq("resolved"),
                        conflicts::updated_at.eq(Utc::now().to_rfc3339()),
                    ))
                    .execute(connection)?;
                append_event(
                    connection,
                    &project_key,
                    None,
                    "conflict_resolved",
                    json!({"conflict_id":conflict_key,"resolution":resolution}),
                )?;
            }
            Ok(true)
        })
    }
}

fn merge_projection(
    connection: &mut DbConnection,
    observation: &ConflictObservation<'_>,
) -> Result<()> {
    let current = find_conflict(connection, observation.conflict_id)?;
    diesel::insert_into(conflicts::table)
        .values(NewConflict {
            id: observation.conflict_id,
            project_id: observation.project_key,
            left_claim_id: &observation.left_claim.id,
            right_claim_id: &observation.right_claim.id,
            path: &observation.advisory.path,
            severity: observation.severity,
            kind: &observation.advisory.kind,
            status: "open",
            message: &observation.advisory.message,
            created_at: observation.now,
            updated_at: observation.now,
        })
        .on_conflict(conflicts::id)
        .do_update()
        .set(conflicts::updated_at.eq(observation.now))
        .execute(connection)?;
    match current {
        Some(previous) if observation.supersedes(&previous) => {
            replace_evidence(connection, observation, &previous)
        }
        _ => Ok(()),
    }
}

impl ConflictObservation<'_> {
    /// An ended overlap that recurs is a new episode; an explicit resolution yields only
    /// to strictly stronger evidence.
    fn supersedes(&self, previous: &ConflictRecord) -> bool {
        previous.status == ConflictStatus::Ended || self.advisory.severity > previous.severity
    }
}

fn replace_evidence(
    connection: &mut DbConnection,
    observation: &ConflictObservation<'_>,
    previous: &ConflictRecord,
) -> Result<()> {
    diesel::update(conflicts::table.filter(conflicts::id.eq(observation.conflict_id)))
        .set((
            conflicts::left_claim_id.eq(&observation.left_claim.id),
            conflicts::right_claim_id.eq(&observation.right_claim.id),
            conflicts::severity.eq(observation.severity),
            conflicts::kind.eq(&observation.advisory.kind),
            conflicts::status.eq("open"),
            conflicts::message.eq(&observation.advisory.message),
            conflicts::updated_at.eq(observation.now),
        ))
        .execute(connection)?;
    if previous.status == ConflictStatus::Open {
        return Ok(());
    }
    append_event(
        connection,
        observation.project_key,
        Some(&observation.left_claim.session_id),
        "conflict_reopened",
        json!({
            "conflict_id": observation.conflict_id,
            "previous_status": previous.status,
            "previous_severity": previous.severity,
            "new_severity": observation.advisory.severity,
        }),
    )
}

pub(super) fn end_stale_conflicts(
    connection: &mut DbConnection,
    project_key: Option<&str>,
) -> Result<usize> {
    let mut query = conflicts::table
        .filter(super::liveness::ended_conflict())
        .into_boxed();
    if let Some(project_key) = project_key {
        query = query.filter(conflicts::project_id.eq(project_key));
    }
    let stale = query
        .order((conflicts::updated_at.asc(), conflicts::id.asc()))
        .limit(i64::try_from(MAX_CONFLICT_CLOSURES)?)
        .select(ConflictRow::as_select())
        .load::<ConflictRow>(connection)?;
    let now = Utc::now();
    for conflict in &stale {
        let ended_at = overlap_end(connection, conflict, now)?.to_rfc3339();
        diesel::update(
            conflicts::table
                .filter(conflicts::id.eq(&conflict.id))
                .filter(conflicts::status.eq("open")),
        )
        .set((
            conflicts::status.eq(ConflictStatus::Ended.to_string()),
            conflicts::updated_at.eq(&ended_at),
        ))
        .execute(connection)?;
        append_event(
            connection,
            &conflict.project_id,
            None,
            "conflict_ended",
            json!({"conflict_id": conflict.id, "ended_at": ended_at}),
        )?;
    }
    Ok(stale.len())
}

/// The overlap ended when the first owner's last claim on the path was released or
/// expired. The result never precedes the last observation or follows the closure.
fn overlap_end(
    connection: &mut DbConnection,
    conflict: &ConflictRow,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>> {
    let observed = parse_time(&conflict.updated_at)?;
    let mut ended: Option<DateTime<Utc>> = None;
    for claim_id in [&conflict.left_claim_id, &conflict.right_claim_id] {
        let owner = claims::table
            .filter(claims::project_id.eq(&conflict.project_id))
            .filter(claims::id.eq(claim_id))
            .select(claims::session_id)
            .first::<String>(connection)
            .optional()?;
        let Some(owner) = owner else { continue };
        if let Some(owner_end) = owner_claim_end(connection, conflict, &owner)? {
            ended = Some(ended.map_or(owner_end, |earlier| earlier.min(owner_end)));
        }
    }
    Ok(ended.unwrap_or(observed).clamp(observed, now.max(observed)))
}

fn owner_claim_end(
    connection: &mut DbConnection,
    conflict: &ConflictRow,
    owner: &str,
) -> Result<Option<DateTime<Utc>>> {
    let owned = || {
        claims::table
            .filter(claims::project_id.eq(&conflict.project_id))
            .filter(claims::session_id.eq(owner))
            .filter(claims::path.eq(&conflict.path))
    };
    let released = owned()
        .filter(claims::state.eq("released"))
        .select(max(claims::updated_at))
        .first::<Option<String>>(connection)?;
    let held = owned()
        .filter(claims::state.eq_any(["claimed", "modified"]))
        .select(max(claims::expires_at))
        .first::<Option<String>>(connection)?;
    let mut latest = None;
    for value in [released, held].into_iter().flatten() {
        let value = parse_time(&value)?;
        latest = Some(latest.map_or(value, |current: DateTime<Utc>| current.max(value)));
    }
    Ok(latest)
}

fn record_occurrence(
    connection: &mut DbConnection,
    observation: &ConflictObservation<'_>,
) -> Result<()> {
    diesel::insert_into(advisories::table)
        .values(NewAdvisory {
            id: &observation.advisory.id,
            project_id: observation.project_key,
            session_id: &observation.left_claim.session_id,
            tool_use_id: observation.tool_key,
            conflict_id: Some(observation.conflict_id),
            severity: observation.severity,
            kind: &observation.advisory.kind,
            path: &observation.advisory.path,
            message: &observation.advisory.message,
            created_at: observation.now,
        })
        .execute(connection)?;
    append_event(
        connection,
        observation.project_key,
        Some(&observation.left_claim.session_id),
        "conflict_detected",
        serde_json::to_value(observation.advisory)?,
    )
}

fn find_conflict(
    connection: &mut DbConnection,
    conflict_key: &str,
) -> Result<Option<ConflictRecord>> {
    conflicts::table
        .filter(conflicts::id.eq(conflict_key))
        .select(ConflictRow::as_select())
        .first::<ConflictRow>(connection)
        .optional()?
        .map(conflict_from_row)
        .transpose()
}

fn conflict_id(
    project_key: &str,
    path: &str,
    left_claim: &Claim,
    right_claim: &Claim,
) -> Result<String> {
    let (first_session, second_session) = if left_claim.session_id < right_claim.session_id {
        (&left_claim.session_id, &right_claim.session_id)
    } else {
        (&right_claim.session_id, &left_claim.session_id)
    };
    let identity = serde_json::to_vec(&(project_key, path, first_session, second_session))?;
    Ok(blake3::hash(&identity).to_hex().to_string())
}
