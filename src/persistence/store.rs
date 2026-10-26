// @feature persistence
// @spec docs/features/persistence.md
// @entrypoint Store::open
// @entrypoint Store::connect
// @boundary dynamic-json
use super::models::{
    claim_from_row, encode_line, parse_time, session_from_row, ClaimRow, EventRow, NewClaim,
    NewEvent, NewSession, SessionRow,
};
use super::schema::{advisories, claims, events, sessions};
use super::window::bounded_projection_reader;
use crate::coordination::domain::{
    Claim, ClaimState, EventRecord, IgnoredAdvisoryPolicy, PathIntent, RecordScope, SessionRecord,
    StatusCounts, ToolCompletion, ToolResultEvent,
};
use crate::persistence::backend::{DbBackend, DbConnection};
use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use diesel::dsl::{count_star, max};
use diesel::prelude::*;
use serde::Serialize;
use serde_json::json;
use std::path::Path;
use uuid::Uuid;

pub(super) const MAX_EVENT_PAYLOAD_BYTES: usize = 64 * 1024;
const MAX_VISIBLE_EVENT_PAYLOAD_BYTES: usize = 4 * 1024;
const MAX_EVENT_SESSION_ID_BYTES: usize = 512;
const MAX_RECONCILABLE_SESSIONS: i64 = 64;

pub struct Store {
    pub(super) connection: DbConnection,
    #[cfg(test)]
    _temporary_database: Option<tempfile::TempDir>,
}

#[derive(Clone, Copy)]
enum ActiveClaimMatch {
    Any,
    Destructive,
    KnownRangeOverlap { start: i32, end: i32 },
    UnknownRange,
}

fn active_claim_row(
    connection: &mut DbConnection,
    project_key: &str,
    session_key: &str,
    intent_path: &str,
    active_at: &str,
    claim_match: ActiveClaimMatch,
) -> Result<Option<ClaimRow>> {
    let mut query = claims::table
        .filter(claims::project_id.eq(project_key))
        .filter(claims::session_id.eq(session_key))
        .filter(claims::path.eq(intent_path))
        .filter(claims::state.eq_any(["claimed", "modified"]))
        .filter(claims::expires_at.gt(active_at))
        .into_boxed::<DbBackend>();
    query = match claim_match {
        ActiveClaimMatch::Any => query,
        ActiveClaimMatch::Destructive => {
            query.filter(claims::operation.eq_any(["delete", "rename"]))
        }
        ActiveClaimMatch::KnownRangeOverlap { start, end } => query
            .filter(claims::line_start.ge(0))
            .filter(claims::line_end.ge(0))
            .filter(claims::line_start.le(end))
            .filter(claims::line_end.ge(start)),
        ActiveClaimMatch::UnknownRange => {
            query.filter(claims::line_start.lt(0).or(claims::line_end.lt(0)))
        }
    };
    Ok(query
        .order((claims::updated_at.desc(), claims::id.asc()))
        .select(ClaimRow::as_select())
        .first::<ClaimRow>(connection)
        .optional()?)
}

fn strongest_active_claim_for_session(
    connection: &mut DbConnection,
    project_key: &str,
    session_key: &str,
    intent: &PathIntent,
    active_at: &str,
) -> Result<Option<Claim>> {
    let find = |connection: &mut DbConnection, claim_match| {
        active_claim_row(
            connection,
            project_key,
            session_key,
            &intent.path,
            active_at,
            claim_match,
        )
    };
    let row = if intent.operation.is_destructive() {
        find(connection, ActiveClaimMatch::Any)?
    } else if let Some(row) = find(connection, ActiveClaimMatch::Destructive)? {
        Some(row)
    } else if let (Some(start), Some(end)) = (intent.line_start, intent.line_end) {
        let overlapping = if let Ok(start) = i32::try_from(start) {
            find(
                connection,
                ActiveClaimMatch::KnownRangeOverlap {
                    start,
                    end: i32::try_from(end).unwrap_or(i32::MAX),
                },
            )?
        } else {
            None
        };
        match overlapping {
            Some(row) => Some(row),
            None => match find(connection, ActiveClaimMatch::UnknownRange)? {
                Some(row) => Some(row),
                None => find(connection, ActiveClaimMatch::Any)?,
            },
        }
    } else {
        find(connection, ActiveClaimMatch::Any)?
    };
    row.map(claim_from_row).transpose()
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        super::migrations::migrate_database(path)?;
        let url = path.to_string_lossy();
        let connection = super::migrations::establish_connection(&url)
            .with_context(|| format!("open {}", path.display()))?;
        Ok(Self {
            connection,
            #[cfg(test)]
            _temporary_database: None,
        })
    }

    pub fn connect(database_url: &str) -> Result<Self> {
        super::migrations::migrate_database_url(database_url)?;
        let connection = super::migrations::establish_connection(database_url)
            .context("open configured database")?;
        Ok(Self {
            connection,
            #[cfg(test)]
            _temporary_database: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn open_memory() -> Result<Self> {
        let temporary = tempfile::tempdir().context("create temporary test database")?;
        let mut store = Self::open(&temporary.path().join("nexus.db"))?;
        store._temporary_database = Some(temporary);
        Ok(store)
    }
}

impl Store {
    pub(crate) fn touch_session(
        &mut self,
        project_key: &str,
        session_key: &str,
        agent_name: &str,
        worktree_path: Option<&str>,
        resolved_config_hash: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.transaction(|connection| {
                let new_session = NewSession {
                    session_id: session_key,
                    project_id: project_key,
                    agent: agent_name,
                    worktree: worktree_path,
                    task_summary: None,
                    prompt_hash: None,
                    status: "active",
                    config_hash: resolved_config_hash,
                    started_at: &now,
                    last_seen_at: &now,
                };
                diesel::insert_into(sessions::table)
                    .values(&new_session)
                    .on_conflict((sessions::project_id, sessions::session_id))
                    .do_update()
                    .set((
                        sessions::agent.eq(agent_name),
                        sessions::worktree.eq(worktree_path),
                        sessions::status.eq("active"),
                        sessions::config_hash.eq(resolved_config_hash),
                        sessions::last_seen_at.eq(&now),
                    ))
                    .execute(connection)?;
                append_event(
                    connection,
                    project_key,
                    Some(session_key),
                    "session_seen",
                    json!({"agent":agent_name,"worktree":worktree_path,"config_hash":resolved_config_hash}),
                )?;
                Ok(())
            })
    }

    pub(crate) fn ensure_session(
        &mut self,
        project_key: &str,
        session_key: &str,
        agent_name: &str,
        resolved_config_hash: &str,
    ) -> Result<()> {
        let exists = sessions::table
            .filter(sessions::session_id.eq(session_key))
            .filter(sessions::project_id.eq(project_key))
            .select(sessions::session_id)
            .first::<String>(&mut self.connection)
            .optional()?
            .is_some();
        if exists {
            return Ok(());
        }
        self.touch_session(
            project_key,
            session_key,
            agent_name,
            None,
            resolved_config_hash,
        )
    }

    pub(crate) fn record_prompt(
        &mut self,
        project_key: &str,
        session_key: &str,
        synopsis_value: Option<&str>,
        hash: &str,
        full_prompt: Option<&str>,
    ) -> Result<()> {
        self.transaction(|connection| {
            diesel::update(
                sessions::table
                    .filter(sessions::project_id.eq(project_key))
                    .filter(sessions::session_id.eq(session_key)),
            )
            .set((
                sessions::task_summary.eq(synopsis_value),
                sessions::prompt_hash.eq(hash),
            ))
            .execute(connection)?;
            append_event(
                connection,
                project_key,
                Some(session_key),
                "user_prompt",
                json!({"synopsis":synopsis_value,"prompt_hash":hash,"full_prompt":full_prompt}),
            )?;
            Ok(())
        })
    }

    pub(crate) fn active_claims_for_path(
        &mut self,
        project_key: &str,
        intent: &PathIntent,
        except_session: &str,
        limit: usize,
    ) -> Result<Vec<Claim>> {
        let limit = limit.min(crate::coordination::api::MAX_LIFECYCLE_ADVISORIES);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let now = Utc::now().to_rfc3339();
        let sessions = claims::table
            .filter(claims::project_id.eq(project_key))
            .filter(claims::path.eq(&intent.path))
            .filter(claims::session_id.ne(except_session))
            .filter(claims::state.eq_any(["claimed", "modified"]))
            .filter(claims::expires_at.gt(&now))
            .group_by(claims::session_id)
            .select((claims::session_id, max(claims::updated_at)))
            .order((max(claims::updated_at).desc(), claims::session_id.asc()))
            .limit(i64::try_from(limit)?)
            .load::<(String, Option<String>)>(&mut self.connection)?;

        let mut selected = Vec::with_capacity(sessions.len());
        for (session_id, _) in sessions {
            if let Some(claim) = strongest_active_claim_for_session(
                &mut self.connection,
                project_key,
                &session_id,
                intent,
                &now,
            )? {
                selected.push(claim);
            }
        }
        Ok(selected)
    }

    pub(crate) fn insert_claim(
        &mut self,
        project_key: &str,
        session_key: &str,
        tool_key: &str,
        intent: &PathIntent,
        ttl_seconds: i64,
    ) -> Result<Claim> {
        let new_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        let expires = now + Duration::seconds(ttl_seconds);
        let expires_text = expires.to_rfc3339();
        let operation_text = intent.operation.to_string();
        let start = encode_line(intent.line_start);
        let end = encode_line(intent.line_end);
        let id = self.transaction(|connection| {
                let new_claim = NewClaim {
                    id: &new_id,
                    project_id: project_key,
                    session_id: session_key,
                    tool_use_id: tool_key,
                    path: &intent.path,
                    operation: &operation_text,
                    line_start: start,
                    line_end: end,
                    state: "claimed",
                    expires_at: &expires_text,
                    updated_at: &now_text,
                };
                diesel::insert_into(claims::table)
                    .values(&new_claim)
                    .on_conflict((
                        claims::project_id,
                        claims::session_id,
                        claims::tool_use_id,
                        claims::path,
                        claims::operation,
                        claims::line_start,
                        claims::line_end,
                    ))
                    .do_update()
                    .set((
                        claims::state.eq("claimed"),
                        claims::expires_at.eq(&expires_text),
                        claims::updated_at.eq(&now_text),
                    ))
                    .execute(connection)?;
                let stored_id = claims::table
                    .filter(claims::project_id.eq(project_key))
                    .filter(claims::session_id.eq(session_key))
                    .filter(claims::tool_use_id.eq(tool_key))
                    .filter(claims::path.eq(&intent.path))
                    .filter(claims::operation.eq(&operation_text))
                    .filter(claims::line_start.eq(start))
                    .filter(claims::line_end.eq(end))
                    .select(claims::id)
                    .first::<String>(connection)?;
                append_event(
                    connection,
                    project_key,
                    Some(session_key),
                    "claim_created",
                    json!({"claim_id":stored_id,"tool_use_id":tool_key,"path":intent.path,"operation":intent.operation,"line_start":intent.line_start,"line_end":intent.line_end,"expires_at":expires}),
                )?;
                Ok(stored_id)
            })?;
        Ok(Claim {
            id,
            project_id: project_key.into(),
            session_id: session_key.into(),
            tool_use_id: tool_key.into(),
            path: intent.path.clone(),
            operation: intent.operation,
            line_start: intent.line_start,
            line_end: intent.line_end,
            state: ClaimState::Claimed,
            expires_at: expires,
            updated_at: now,
        })
    }

    pub(crate) fn record_tool_inspection(
        &mut self,
        project_key: &str,
        session_key: &str,
        tool_key: &str,
        tool_name: &str,
        intents: &[PathIntent],
    ) -> Result<()> {
        append_event(
            &mut self.connection,
            project_key,
            Some(session_key),
            "tool_inspected",
            json!({"tool_use_id":tool_key,"tool_name":tool_name,"intents":intents}),
        )
    }

    pub(crate) fn record_hook_result(
        &mut self,
        project_key: &str,
        session_key: &str,
        tool_key: &str,
        completion: ToolCompletion,
        payload: ToolResultEvent,
        ignored_advisory_policy: IgnoredAdvisoryPolicy,
    ) -> Result<()> {
        let event_kind = match completion {
            ToolCompletion::Succeeded => "tool_succeeded",
            ToolCompletion::Failed => "tool_failed",
        };
        self.transaction(|connection| {
            append_event(
                connection,
                project_key,
                Some(session_key),
                event_kind,
                payload,
            )?;
            if completion == ToolCompletion::Succeeded {
                diesel::update(
                    claims::table
                        .filter(claims::project_id.eq(project_key))
                        .filter(claims::session_id.eq(session_key))
                        .filter(claims::tool_use_id.eq(tool_key)),
                )
                .set((
                    claims::state.eq("modified"),
                    claims::updated_at.eq(Utc::now().to_rfc3339()),
                ))
                .execute(connection)?;
                let advisory_count = advisories::table
                    .filter(advisories::project_id.eq(project_key))
                    .filter(advisories::session_id.eq(session_key))
                    .filter(advisories::tool_use_id.eq(tool_key))
                    .select(count_star())
                    .first::<i64>(connection)?;
                if ignored_advisory_policy == IgnoredAdvisoryPolicy::Record && advisory_count > 0 {
                    append_event(
                        connection,
                        project_key,
                        Some(session_key),
                        "advisory_ignored",
                        json!({"tool_use_id":tool_key,"advisory_count":advisory_count}),
                    )?;
                }
            }
            Ok(())
        })
    }

    pub(crate) fn stop_session(&mut self, project_key: &str, session_key: &str) -> Result<()> {
        self.transaction(|connection| {
            let now = Utc::now().to_rfc3339();
            diesel::update(
                sessions::table
                    .filter(sessions::project_id.eq(project_key))
                    .filter(sessions::session_id.eq(session_key)),
            )
            .set((
                sessions::status.eq("stopped"),
                sessions::last_seen_at.eq(&now),
            ))
            .execute(connection)?;
            diesel::update(
                claims::table
                    .filter(claims::project_id.eq(project_key))
                    .filter(claims::session_id.eq(session_key))
                    .filter(claims::state.eq_any(["claimed", "modified"])),
            )
            .set((claims::state.eq("released"), claims::updated_at.eq(&now)))
            .execute(connection)?;
            append_event(
                connection,
                project_key,
                Some(session_key),
                "session_stopped",
                json!({}),
            )?;
            Ok(())
        })
    }
}

impl Store {
    pub fn list_events(
        &mut self,
        project_key: Option<&str>,
        maximum: usize,
    ) -> Result<Vec<EventRecord>> {
        self.load_event_window(project_key, maximum)
            .map(|(events, _)| events)
    }

    pub(super) fn load_event_window(
        &mut self,
        project_key: Option<&str>,
        maximum: usize,
    ) -> Result<(Vec<EventRecord>, bool)> {
        let maximum = maximum.min(crate::coordination::api::MAX_EVENT_RESULTS);
        let mut query = events::table.into_boxed();
        if let Some(project_key) = project_key {
            query = query.filter(events::project_id.eq(project_key));
        }
        let mut rows = query
            .order(events::id.desc())
            .limit(i64::try_from(maximum.saturating_add(1)).unwrap_or(i64::MAX))
            .select(EventRow::as_select())
            .load::<EventRow>(&mut self.connection)?;
        let truncated = rows.len() > maximum;
        rows.truncate(maximum);
        let records = rows
            .into_iter()
            .map(event_from_row)
            .collect::<Result<_>>()?;
        Ok((records, truncated))
    }

    pub fn counts(&mut self, activity_window_seconds: i64) -> Result<StatusCounts> {
        self.dashboard_counts(None, activity_window_seconds)
    }

    pub(crate) fn reconcilable_sessions(
        &mut self,
        activity_window_seconds: i64,
    ) -> Result<Vec<SessionRecord>> {
        let cutoff = (Utc::now() - Duration::seconds(activity_window_seconds)).to_rfc3339();
        sessions::table
            .filter(sessions::status.eq("active"))
            .filter(sessions::worktree.is_not_null())
            .filter(sessions::last_seen_at.gt(cutoff))
            .order((
                sessions::last_seen_at.desc(),
                sessions::project_id.asc(),
                sessions::session_id.asc(),
            ))
            .limit(MAX_RECONCILABLE_SESSIONS)
            .select(SessionRow::as_select())
            .load::<SessionRow>(&mut self.connection)?
            .into_iter()
            .map(session_from_row)
            .collect()
    }

    pub fn list_sessions(
        &mut self,
        scope: RecordScope,
        activity_window_seconds: i64,
    ) -> Result<Vec<SessionRecord>> {
        Ok(self
            .list_sessions_window(None, scope, activity_window_seconds)?
            .items)
    }

    bounded_projection_reader! {
        pub(crate) fn list_sessions_window(project_key, scope: RecordScope, activity_window_seconds: i64) -> SessionRecord {
            table: sessions::table,
            project_column: sessions::project_id,
            scoped_when: scope == RecordScope::Active,
            scoped_filter: super::liveness::recent_session(activity_window_seconds),
            order: (
                sessions::last_seen_at.desc(),
                sessions::project_id.asc(),
                sessions::session_id.asc(),
            ),
            row: SessionRow,
            decode: session_from_row,
            maximum: crate::coordination::api::MAX_SESSION_RESULTS,
        }
    }

    pub fn list_claims(
        &mut self,
        project_key: Option<&str>,
        scope: RecordScope,
    ) -> Result<Vec<Claim>> {
        Ok(self.list_claims_window(project_key, scope)?.items)
    }

    bounded_projection_reader! {
        pub(crate) fn list_claims_window(project_key, scope: RecordScope) -> Claim {
            table: claims::table,
            project_column: claims::project_id,
            scoped_when: scope == RecordScope::Active,
            scoped_filter: claims::state
                .eq_any(["claimed", "modified"])
                .and(claims::expires_at.gt(Utc::now().to_rfc3339())),
            order: (claims::updated_at.desc(), claims::id.asc()),
            row: ClaimRow,
            decode: claim_from_row,
            maximum: crate::coordination::api::MAX_CLAIM_RESULTS,
        }
    }
}

impl Store {
    pub(crate) fn record_reconciliation(
        &mut self,
        project_key: &str,
        session_key: &str,
        intents: &[PathIntent],
        ttl_seconds: i64,
    ) -> Result<()> {
        if !active_session_exists(&mut self.connection, project_key, session_key)? {
            return Ok(());
        }
        let mut newly_observed = Vec::new();
        for intent in intents {
            if self.refresh_reconciled_claim(project_key, session_key, intent, ttl_seconds)? {
                newly_observed.push(intent);
            }
        }
        if !newly_observed.is_empty() {
            append_event(
                &mut self.connection,
                project_key,
                Some(session_key),
                "git_reconciled",
                json!({"changed_paths":newly_observed}),
            )?;
        }
        Ok(())
    }

    fn refresh_reconciled_claim(
        &mut self,
        project_key: &str,
        session_key: &str,
        intent: &PathIntent,
        ttl_seconds: i64,
    ) -> Result<bool> {
        let tool_key = format!(
            "git-reconcile:{}",
            blake3::hash(intent.path.as_bytes()).to_hex()
        );
        let existing = self.reconciled_claim(project_key, session_key, intent, &tool_key)?;
        let now = Utc::now();
        let is_active = existing.as_ref().is_some_and(|claim| {
            matches!(claim.state.as_str(), "claimed" | "modified")
                && parse_time(&claim.expires_at).is_ok_and(|expires| expires > now)
        });
        if let Some(claim) = existing.filter(|_| is_active) {
            diesel::update(claims::table.filter(claims::id.eq(claim.id)))
                .set((
                    claims::expires_at.eq((now + Duration::seconds(ttl_seconds)).to_rfc3339()),
                    claims::updated_at.eq(now.to_rfc3339()),
                ))
                .execute(&mut self.connection)?;
            return Ok(false);
        }
        self.insert_claim(project_key, session_key, &tool_key, intent, ttl_seconds)?;
        Ok(true)
    }

    fn reconciled_claim(
        &mut self,
        project_key: &str,
        session_key: &str,
        intent: &PathIntent,
        tool_key: &str,
    ) -> Result<Option<ClaimRow>> {
        Ok(claims::table
            .filter(claims::project_id.eq(project_key))
            .filter(claims::session_id.eq(session_key))
            .filter(claims::tool_use_id.eq(tool_key))
            .filter(claims::path.eq(&intent.path))
            .filter(claims::operation.eq(intent.operation.to_string()))
            .filter(claims::line_start.eq(encode_line(intent.line_start)))
            .filter(claims::line_end.eq(encode_line(intent.line_end)))
            .select(ClaimRow::as_select())
            .first::<ClaimRow>(&mut self.connection)
            .optional()?)
    }
}

fn active_session_exists(
    connection: &mut DbConnection,
    project_key: &str,
    session_key: &str,
) -> Result<bool> {
    Ok(sessions::table
        .filter(sessions::project_id.eq(project_key))
        .filter(sessions::session_id.eq(session_key))
        .filter(sessions::status.eq("active"))
        .select(sessions::session_id)
        .first::<String>(connection)
        .optional()?
        .is_some())
}

pub(super) fn append_event(
    connection: &mut DbConnection,
    project_key: &str,
    session_key: Option<&str>,
    event_kind: &str,
    payload: impl Serialize,
) -> Result<()> {
    let payload_json = bounded_event_payload(&payload)?;
    let bounded_session_key = session_key.map(|value| {
        if value.len() <= MAX_EVENT_SESSION_ID_BYTES {
            value.to_owned()
        } else {
            format!(
                "oversized-session:{}",
                blake3::hash(value.as_bytes()).to_hex()
            )
        }
    });
    diesel::insert_into(events::table)
        .values(NewEvent {
            project_id: project_key,
            session_id: bounded_session_key.as_deref(),
            kind: event_kind,
            payload_json,
            created_at: Utc::now().to_rfc3339(),
        })
        .execute(connection)?;
    Ok(())
}

fn bounded_event_payload(payload: &impl Serialize) -> Result<String> {
    let encoded = serde_json::to_string(payload)?;
    if encoded.len() <= MAX_EVENT_PAYLOAD_BYTES {
        return Ok(encoded);
    }
    Ok(serde_json::to_string(&json!({
        "truncated": true,
        "reason": "event_payload_too_large",
        "original_bytes": encoded.len(),
        "digest": blake3::hash(encoded.as_bytes()).to_hex().to_string(),
    }))?)
}

fn event_from_row(row: EventRow) -> Result<EventRecord> {
    let visible_payload = if row.payload_json.len() <= MAX_VISIBLE_EVENT_PAYLOAD_BYTES {
        row.payload_json
    } else {
        serde_json::to_string(&json!({
            "truncated": true,
            "reason": "event_payload_omitted",
            "original_bytes": row.payload_json.len(),
            "digest": blake3::hash(row.payload_json.as_bytes()).to_hex().to_string(),
        }))?
    };
    Ok(EventRecord {
        id: row.id,
        project_id: row.project_id,
        session_id: row.session_id,
        kind: row.kind,
        payload: serde_json::from_str::<serde_json::Value>(&visible_payload)
            .map(crate::coordination::domain::EventPayload::from)?,
        created_at: parse_time(&row.created_at)?,
    })
}

#[cfg(all(test, feature = "sqlite"))]
mod tests;
