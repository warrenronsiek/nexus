// @feature persistence
// @feature coordination
// @spec docs/features/persistence.md
// @spec docs/features/coordination.md
//! One session's coordination history: the conflicts it is party to, the advisories Nexus
//! delivered about them, and the sessions on the other side.
use super::backend::DbConnection;
use super::models::{parse_time, session_from_row, ConflictRow, SessionRow};
use super::schema::{advisories, claims, conflicts, sessions};
use super::session_names::host_title;
use super::Store;
use crate::coordination::domain::{
    query_window_limit, ConflictStatus, RecordWindow, SessionCounterpart, SessionRecord,
    SessionTimeline, Severity, TimelineDirection, TimelineEntry, TimelineEntryKind,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::str::FromStr;

mod index;

#[derive(Queryable, Selectable)]
#[diesel(table_name = advisories)]
struct AdvisoryRow {
    id: String,
    session_id: String,
    conflict_id: Option<String>,
    severity: String,
    path: String,
    message: String,
    created_at: String,
}

impl Store {
    /// The named counterpart used in advisory messages and timelines.
    pub(crate) fn session_counterpart(
        &mut self,
        project_key: &str,
        session_key: &str,
    ) -> Result<SessionCounterpart> {
        let mut named = counterparts(
            &mut self.connection,
            project_key,
            &BTreeSet::from([session_key.to_owned()]),
        )?;
        Ok(named
            .remove(session_key)
            .unwrap_or_else(|| SessionCounterpart::unnamed(session_key)))
    }

    pub(crate) fn session_timeline(
        &mut self,
        project_key: &str,
        session_key: &str,
        limit: usize,
    ) -> Result<SessionTimeline> {
        let connection = &mut self.connection;
        let Some(session) = session_record(connection, project_key, session_key)? else {
            return Ok(SessionTimeline {
                session: None,
                entries: Vec::new(),
                truncated: false,
            });
        };
        let scope = TimelineScope {
            project: project_key,
            session: session_key,
            limit,
        };
        let sources = TimelineSources::load(connection, &scope)?;
        let (entries, truncated) = sources.into_entries(connection, &scope)?;
        Ok(SessionTimeline {
            session: Some(session),
            entries,
            truncated,
        })
    }
}

struct TimelineScope<'a> {
    project: &'a str,
    session: &'a str,
    limit: usize,
}

/// The newest bounded rows from each timeline source.
struct TimelineSources {
    conflicts: RecordWindow<ConflictRow>,
    advisories: RecordWindow<AdvisoryRow>,
}

impl TimelineSources {
    fn load(connection: &mut DbConnection, scope: &TimelineScope<'_>) -> Result<Self> {
        Ok(Self {
            conflicts: RecordWindow::bounded(party_conflict_rows(connection, scope)?, scope.limit),
            advisories: RecordWindow::bounded(
                timeline_advisory_rows(connection, scope)?,
                scope.limit,
            ),
        })
    }

    /// Either source may have omitted older rows; only the span both cover is complete.
    fn cutoff(&self) -> Result<Option<DateTime<Utc>>> {
        let conflict = self
            .conflicts
            .truncated
            .then(|| self.conflicts.items.last().map(|row| &row.updated_at))
            .flatten();
        let advisory = self
            .advisories
            .truncated
            .then(|| self.advisories.items.last().map(|row| &row.created_at))
            .flatten();
        let mut cutoff = None;
        for value in [conflict, advisory].into_iter().flatten() {
            cutoff = cutoff.max(Some(parse_time(value)?));
        }
        Ok(cutoff)
    }

    fn into_entries(
        self,
        connection: &mut DbConnection,
        scope: &TimelineScope<'_>,
    ) -> Result<(Vec<TimelineEntry>, bool)> {
        let cutoff = self.cutoff()?;
        let truncated = self.conflicts.truncated || self.advisories.truncated;
        let context = TimelineContext::load(
            connection,
            scope,
            self.conflicts.items,
            &self.advisories.items,
        )?;
        let mut entries = context.entries(scope.session, &self.advisories.items)?;
        if let Some(cutoff) = cutoff {
            entries.retain(|entry| entry.at >= cutoff);
        }
        entries.sort_by(|left, right| {
            (left.at, left.kind, &left.id).cmp(&(right.at, right.kind, &right.id))
        });
        let excess = entries.len().saturating_sub(scope.limit);
        entries.drain(..excess);
        Ok((entries, truncated || excess > 0))
    }
}

/// Conflicts with their owner sessions and the named counterparts they reference.
struct TimelineContext {
    conflicts: Vec<PartyConflict>,
    /// Conflicts the session is party to; later rows were loaded only for advisories.
    primary: usize,
    names: BTreeMap<String, SessionCounterpart>,
}

impl TimelineContext {
    fn load(
        connection: &mut DbConnection,
        scope: &TimelineScope<'_>,
        mut rows: Vec<ConflictRow>,
        advisories: &[AdvisoryRow],
    ) -> Result<Self> {
        let loaded = rows
            .iter()
            .map(|row| row.id.clone())
            .collect::<BTreeSet<_>>();
        let referenced = advisories
            .iter()
            .filter_map(|row| row.conflict_id.clone())
            .filter(|id| !loaded.contains(id))
            .collect::<BTreeSet<_>>();
        let primary = rows.len();
        rows.extend(conflicts_by_id(connection, scope.project, &referenced)?);
        let conflicts = with_owners(connection, scope.project, rows)?;
        let wanted = timeline_counterparts(scope.session, &conflicts, advisories);
        Ok(Self {
            names: counterparts(connection, scope.project, &wanted)?,
            conflicts,
            primary,
        })
    }

    fn entries(&self, session: &str, advisories: &[AdvisoryRow]) -> Result<Vec<TimelineEntry>> {
        let by_id = self
            .conflicts
            .iter()
            .map(|conflict| (conflict.row.id.as_str(), conflict))
            .collect::<HashMap<_, _>>();
        let mut entries = Vec::new();
        for conflict in &self.conflicts[..self.primary] {
            entries.extend(conflict.entries(session, &self.names)?);
        }
        for row in advisories {
            let conflict = row.conflict_id.as_deref().and_then(|id| by_id.get(id));
            entries.push(advisory_entry(
                session,
                row,
                conflict.copied(),
                &self.names,
            )?);
        }
        Ok(entries)
    }
}

/// A conflict with its owner sessions resolved from the original claim rows.
struct PartyConflict {
    row: ConflictRow,
    left: Option<String>,
    right: Option<String>,
}

impl PartyConflict {
    fn other_than(&self, session: &str) -> Option<&str> {
        if self.left.as_deref() == Some(session) {
            self.right.as_deref()
        } else {
            self.left.as_deref()
        }
    }

    fn status(&self) -> Result<ConflictStatus> {
        ConflictStatus::from_str(&self.row.status).map_err(anyhow::Error::msg)
    }

    /// The detecting hook notified the owner of the left claim.
    fn direction(&self, session: &str) -> TimelineDirection {
        if self.left.as_deref() == Some(session) {
            TimelineDirection::Received
        } else {
            TimelineDirection::Sent
        }
    }

    fn closure_message(&self, status: ConflictStatus) -> Option<String> {
        let path = &self.row.path;
        match status {
            ConflictStatus::Open => None,
            ConflictStatus::Resolved => Some(format!("conflict on {path} was resolved")),
            ConflictStatus::Ended => Some(format!(
                "overlap on {path} ended when a claim was released or expired"
            )),
        }
    }

    fn entries(
        &self,
        session: &str,
        names: &BTreeMap<String, SessionCounterpart>,
    ) -> Result<Vec<TimelineEntry>> {
        let status = self.status()?;
        let detected = TimelineEntry {
            id: format!("{}:detected", self.row.id),
            at: parse_time(&self.row.created_at)?,
            kind: TimelineEntryKind::ConflictDetected,
            path: self.row.path.clone(),
            severity: Severity::from_str(&self.row.severity).map_err(anyhow::Error::msg)?,
            message: self.row.message.clone(),
            direction: self.direction(session),
            counterpart: counterpart(names, self.other_than(session)),
            conflict_id: Some(self.row.id.clone()),
            status: Some(status),
        };
        let Some(message) = self.closure_message(status) else {
            return Ok(vec![detected]);
        };
        let resolved = TimelineEntry {
            id: format!("{}:resolved", self.row.id),
            at: parse_time(&self.row.updated_at)?,
            kind: TimelineEntryKind::ConflictResolved,
            message,
            ..detected.clone()
        };
        Ok(vec![detected, resolved])
    }
}

fn advisory_entry(
    session: &str,
    row: &AdvisoryRow,
    conflict: Option<&PartyConflict>,
    names: &BTreeMap<String, SessionCounterpart>,
) -> Result<TimelineEntry> {
    let (direction, other) = if row.session_id == session {
        (
            TimelineDirection::Received,
            conflict.and_then(|conflict| conflict.other_than(session)),
        )
    } else {
        (TimelineDirection::Sent, Some(row.session_id.as_str()))
    };
    Ok(TimelineEntry {
        id: row.id.clone(),
        at: parse_time(&row.created_at)?,
        kind: TimelineEntryKind::Advisory,
        path: row.path.clone(),
        severity: Severity::from_str(&row.severity).map_err(anyhow::Error::msg)?,
        message: row.message.clone(),
        direction,
        counterpart: counterpart(names, other),
        conflict_id: row.conflict_id.clone(),
        status: conflict.map(PartyConflict::status).transpose()?,
    })
}

/// Conflicts whose original left or right claim belongs to the session, newest first.
fn party_conflict_rows(
    connection: &mut DbConnection,
    scope: &TimelineScope<'_>,
) -> Result<Vec<ConflictRow>> {
    let owned = || {
        claims::table
            .filter(claims::project_id.eq(scope.project))
            .filter(claims::session_id.eq(scope.session))
            .select(claims::id)
    };
    Ok(conflicts::table
        .filter(conflicts::project_id.eq(scope.project))
        .filter(
            conflicts::left_claim_id
                .eq_any(owned())
                .or(conflicts::right_claim_id.eq_any(owned())),
        )
        .order((conflicts::updated_at.desc(), conflicts::id.asc()))
        .limit(query_window_limit(scope.limit))
        .select(ConflictRow::as_select())
        .load(connection)?)
}

/// Advisories delivered to the session, plus those delivered to a counterpart about a
/// conflict the session is party to, newest first.
fn timeline_advisory_rows(
    connection: &mut DbConnection,
    scope: &TimelineScope<'_>,
) -> Result<Vec<AdvisoryRow>> {
    let owned = || {
        claims::table
            .filter(claims::project_id.eq(scope.project))
            .filter(claims::session_id.eq(scope.session))
            .select(claims::id)
    };
    let party_conflicts = conflicts::table
        .filter(conflicts::project_id.eq(scope.project))
        .filter(
            conflicts::left_claim_id
                .eq_any(owned())
                .or(conflicts::right_claim_id.eq_any(owned())),
        )
        .select(conflicts::id.nullable());
    Ok(advisories::table
        .filter(advisories::project_id.eq(scope.project))
        .filter(
            advisories::session_id
                .eq(scope.session)
                .or(advisories::conflict_id.eq_any(party_conflicts)),
        )
        .order((advisories::created_at.desc(), advisories::id.asc()))
        .limit(query_window_limit(scope.limit))
        .select(AdvisoryRow::as_select())
        .load(connection)?)
}

fn conflicts_by_id(
    connection: &mut DbConnection,
    project_key: &str,
    ids: &BTreeSet<String>,
) -> Result<Vec<ConflictRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(conflicts::table
        .filter(conflicts::project_id.eq(project_key))
        .filter(conflicts::id.eq_any(ids))
        .select(ConflictRow::as_select())
        .load(connection)?)
}

fn with_owners(
    connection: &mut DbConnection,
    project_key: &str,
    rows: Vec<ConflictRow>,
) -> Result<Vec<PartyConflict>> {
    let claim_ids = rows
        .iter()
        .flat_map(|row| [row.left_claim_id.clone(), row.right_claim_id.clone()])
        .collect::<BTreeSet<_>>();
    let owners = if claim_ids.is_empty() {
        HashMap::new()
    } else {
        claims::table
            .filter(claims::project_id.eq(project_key))
            .filter(claims::id.eq_any(&claim_ids))
            .select((claims::id, claims::session_id))
            .load::<(String, String)>(connection)?
            .into_iter()
            .collect::<HashMap<_, _>>()
    };
    Ok(rows
        .into_iter()
        .map(|row| PartyConflict {
            left: owners.get(&row.left_claim_id).cloned(),
            right: owners.get(&row.right_claim_id).cloned(),
            row,
        })
        .collect())
}

fn timeline_counterparts(
    session: &str,
    conflicts: &[PartyConflict],
    advisories: &[AdvisoryRow],
) -> BTreeSet<String> {
    let recipients = advisories
        .iter()
        .map(|row| row.session_id.as_str())
        .filter(|recipient| *recipient != session);
    conflicts
        .iter()
        .filter_map(|conflict| conflict.other_than(session))
        .chain(recipients)
        .map(str::to_owned)
        .collect()
}

fn counterparts(
    connection: &mut DbConnection,
    project_key: &str,
    ids: &BTreeSet<String>,
) -> Result<BTreeMap<String, SessionCounterpart>> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows = sessions::table
        .filter(sessions::project_id.eq(project_key))
        .filter(sessions::session_id.eq_any(ids))
        .select((sessions::session_id, sessions::agent))
        .load::<(String, String)>(connection)?;
    let mut named = BTreeMap::new();
    for (session_id, agent) in rows {
        let display_name = host_title(connection, project_key, &agent, &session_id)?;
        named.insert(
            session_id.clone(),
            SessionCounterpart {
                session_id,
                agent,
                display_name,
            },
        );
    }
    Ok(named)
}

fn counterpart(
    names: &BTreeMap<String, SessionCounterpart>,
    session: Option<&str>,
) -> Option<SessionCounterpart> {
    session.map(|session| {
        names
            .get(session)
            .cloned()
            .unwrap_or_else(|| SessionCounterpart::unnamed(session))
    })
}

fn session_record(
    connection: &mut DbConnection,
    project_key: &str,
    session_key: &str,
) -> Result<Option<SessionRecord>> {
    let Some(row) = sessions::table
        .filter(sessions::project_id.eq(project_key))
        .filter(sessions::session_id.eq(session_key))
        .select(SessionRow::as_select())
        .first::<SessionRow>(connection)
        .optional()?
    else {
        return Ok(None);
    };
    let mut session = session_from_row(row)?;
    session.display_name = display_name(connection, &session)?;
    Ok(Some(session))
}

fn display_name(connection: &mut DbConnection, session: &SessionRecord) -> Result<Option<String>> {
    host_title(
        connection,
        &session.project_id,
        &session.agent,
        &session.session_id,
    )
}

#[cfg(test)]
#[path = "session_history_tests.rs"]
mod tests;
