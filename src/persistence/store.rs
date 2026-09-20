// @feature persistence
// @spec docs/features/persistence.md
// @entrypoint Store::open
// @boundary dynamic-json
use super::models::{
    claim_from_row, encode_line, parse_time, session_from_row, ClaimRow, EventRow, NewClaim,
    NewEvent, NewSession, SessionRow,
};
use super::schema::{advisories, claims, conflicts, events, sessions};
use crate::coordination::domain::{
    Claim, ClaimRelease, ClaimState, EventRecord, IgnoredAdvisoryPolicy, PathIntent, RecordScope,
    SessionRecord, StatusCounts, ToolCompletion, ToolResultEvent,
};
use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use diesel::dsl::count_star;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde::Serialize;
use serde_json::json;
use std::path::Path;
use uuid::Uuid;

pub struct Store {
    pub(super) connection: SqliteConnection,
    #[cfg(test)]
    _temporary_database: Option<tempfile::TempDir>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        super::migrations::migrate_database(path)?;
        let url = path.to_string_lossy();
        let connection = SqliteConnection::establish(&url)
            .with_context(|| format!("open {}", path.display()))?;
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

    pub(crate) fn touch_session(
        &mut self,
        project_key: &str,
        session_key: &str,
        agent_name: &str,
        worktree_path: Option<&str>,
        resolved_config_hash: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.connection
            .transaction::<_, anyhow::Error, _>(|connection| {
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
                    .on_conflict(sessions::session_id)
                    .do_update()
                    .set((
                        sessions::project_id.eq(project_key),
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

    pub(crate) fn record_prompt(
        &mut self,
        project_key: &str,
        session_key: &str,
        synopsis_value: Option<&str>,
        hash: &str,
        full_prompt: Option<&str>,
    ) -> Result<()> {
        self.connection
            .transaction::<_, anyhow::Error, _>(|connection| {
                diesel::update(sessions::table.filter(sessions::session_id.eq(session_key)))
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
        claim_path: &str,
        except_session: &str,
    ) -> Result<Vec<Claim>> {
        let rows = claims::table
            .filter(claims::project_id.eq(project_key))
            .filter(claims::path.eq(claim_path))
            .filter(claims::session_id.ne(except_session))
            .filter(claims::state.eq_any(["claimed", "modified"]))
            .filter(claims::expires_at.gt(Utc::now().to_rfc3339()))
            .select(ClaimRow::as_select())
            .load(&mut self.connection)?;
        rows.into_iter().map(claim_from_row).collect()
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
        let id = self
            .connection
            .transaction::<_, anyhow::Error, _>(|connection| {
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
        self.connection
            .transaction::<_, anyhow::Error, _>(|connection| {
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
                            .filter(claims::session_id.eq(session_key))
                            .filter(claims::tool_use_id.eq(tool_key)),
                    )
                    .set((
                        claims::state.eq("modified"),
                        claims::updated_at.eq(Utc::now().to_rfc3339()),
                    ))
                    .execute(connection)?;
                    let advisory_count = advisories::table
                        .filter(advisories::session_id.eq(session_key))
                        .filter(advisories::tool_use_id.eq(tool_key))
                        .select(count_star())
                        .first::<i64>(connection)?;
                    if ignored_advisory_policy == IgnoredAdvisoryPolicy::Record
                        && advisory_count > 0
                    {
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
        self.connection
            .transaction::<_, anyhow::Error, _>(|connection| {
                let now = Utc::now().to_rfc3339();
                diesel::update(sessions::table.filter(sessions::session_id.eq(session_key)))
                    .set((
                        sessions::status.eq("stopped"),
                        sessions::last_seen_at.eq(&now),
                    ))
                    .execute(connection)?;
                diesel::update(
                    claims::table
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

    pub fn list_events(
        &mut self,
        project_key: Option<&str>,
        maximum: usize,
    ) -> Result<Vec<EventRecord>> {
        let mut query = events::table.into_boxed();
        if let Some(project_key) = project_key {
            query = query.filter(events::project_id.eq(project_key));
        }
        let rows = query
            .order(events::id.desc())
            .limit(maximum as i64)
            .select(EventRow::as_select())
            .load(&mut self.connection)?;
        rows.into_iter()
            .map(|row| {
                Ok(EventRecord {
                    id: row.id,
                    project_id: row.project_id,
                    session_id: row.session_id,
                    kind: row.kind,
                    payload: serde_json::from_str::<serde_json::Value>(&row.payload_json)
                        .map(crate::coordination::domain::EventPayload::from)?,
                    created_at: parse_time(&row.created_at)?,
                })
            })
            .collect()
    }

    pub fn counts(&mut self) -> Result<StatusCounts> {
        let session_count = sessions::table
            .filter(sessions::status.eq("active"))
            .select(count_star())
            .first::<i64>(&mut self.connection)?;
        let claim_count = claims::table
            .filter(claims::state.eq_any(["claimed", "modified"]))
            .filter(claims::expires_at.gt(Utc::now().to_rfc3339()))
            .select(count_star())
            .first::<i64>(&mut self.connection)?;
        let conflict_count = conflicts::table
            .filter(conflicts::status.eq("open"))
            .select(count_star())
            .first::<i64>(&mut self.connection)?;
        let event_count = events::table
            .select(count_star())
            .first::<i64>(&mut self.connection)?;
        Ok(StatusCounts {
            active_sessions: session_count,
            active_claims: claim_count,
            open_conflicts: conflict_count,
            events: event_count,
        })
    }

    pub(crate) fn active_sessions(&mut self) -> Result<Vec<SessionRecord>> {
        sessions::table
            .filter(sessions::status.eq("active"))
            .filter(sessions::worktree.is_not_null())
            .select(SessionRow::as_select())
            .load::<SessionRow>(&mut self.connection)?
            .into_iter()
            .map(session_from_row)
            .collect()
    }

    pub fn list_sessions(&mut self, scope: RecordScope) -> Result<Vec<SessionRecord>> {
        let mut query = sessions::table.into_boxed();
        if scope == RecordScope::Active {
            query = query.filter(sessions::status.eq("active"));
        }
        let rows = query
            .order(sessions::last_seen_at.desc())
            .select(SessionRow::as_select())
            .load::<SessionRow>(&mut self.connection)?;
        rows.into_iter().map(session_from_row).collect()
    }

    pub fn list_claims(
        &mut self,
        project_key: Option<&str>,
        scope: RecordScope,
    ) -> Result<Vec<Claim>> {
        let mut query = claims::table.into_boxed();
        if let Some(project_key) = project_key {
            query = query.filter(claims::project_id.eq(project_key));
        }
        if scope == RecordScope::Active {
            query = query
                .filter(claims::state.eq_any(["claimed", "modified"]))
                .filter(claims::expires_at.gt(Utc::now().to_rfc3339()));
        }
        let rows = query
            .order(claims::updated_at.desc())
            .select(ClaimRow::as_select())
            .load::<ClaimRow>(&mut self.connection)?;
        rows.into_iter().map(claim_from_row).collect()
    }

    pub(crate) fn release_claims(
        &mut self,
        session_key: &str,
        release: &ClaimRelease,
    ) -> Result<usize> {
        self.connection
            .transaction::<_, anyhow::Error, _>(|connection| {
                let project_key = sessions::table
                    .filter(sessions::session_id.eq(session_key))
                    .select(sessions::project_id)
                    .first::<String>(connection)
                    .optional()?;
                let now = Utc::now().to_rfc3339();
                let count = if let ClaimRelease::Path(path_filter) = release {
                    diesel::update(
                        claims::table
                            .filter(claims::session_id.eq(session_key))
                            .filter(claims::path.eq(path_filter))
                            .filter(claims::state.eq_any(["claimed", "modified"])),
                    )
                    .set((claims::state.eq("released"), claims::updated_at.eq(&now)))
                    .execute(connection)?
                } else {
                    diesel::update(
                        claims::table
                            .filter(claims::session_id.eq(session_key))
                            .filter(claims::state.eq_any(["claimed", "modified"])),
                    )
                    .set((claims::state.eq("released"), claims::updated_at.eq(&now)))
                    .execute(connection)?
                };
                if let Some(project_key) = project_key {
                    append_event(
                        connection,
                        &project_key,
                        Some(session_key),
                        "claims_released",
                        json!({
                            "path":match release {
                                ClaimRelease::Session => None,
                                ClaimRelease::Path(path) => Some(path),
                            },
                            "count":count
                        }),
                    )?;
                }
                Ok(count)
            })
    }

    pub(crate) fn record_reconciliation(
        &mut self,
        session: &SessionRecord,
        intents: &[PathIntent],
        ttl_seconds: i64,
    ) -> Result<()> {
        for intent in intents {
            let tool_key = format!(
                "git-reconcile:{}",
                blake3::hash(intent.path.as_bytes()).to_hex()
            );
            self.insert_claim(
                &session.project_id,
                &session.session_id,
                &tool_key,
                intent,
                ttl_seconds,
            )?;
        }
        if !intents.is_empty() {
            append_event(
                &mut self.connection,
                &session.project_id,
                Some(&session.session_id),
                "git_reconciled",
                json!({"changed_paths":intents}),
            )?;
        }
        Ok(())
    }
}

pub(super) fn append_event(
    connection: &mut SqliteConnection,
    project_key: &str,
    session_key: Option<&str>,
    event_kind: &str,
    payload: impl Serialize,
) -> Result<()> {
    diesel::insert_into(events::table)
        .values(NewEvent {
            project_id: project_key,
            session_id: session_key,
            kind: event_kind,
            payload_json: serde_json::to_string(&payload)?,
            created_at: Utc::now().to_rfc3339(),
        })
        .execute(connection)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flyway_migrations_create_a_queryable_store() {
        let mut store = Store::open_memory().unwrap();
        assert_eq!(store.counts().unwrap().events, 0);
        assert!(store
            .list_claims(None, RecordScope::All)
            .unwrap()
            .is_empty());
    }
}
