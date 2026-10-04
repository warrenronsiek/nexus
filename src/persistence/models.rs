// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
use super::schema::{advisories, capability_uses, claims, conflicts, events, sessions};
use crate::coordination::domain::{
    Claim, ClaimState, ConflictRecord, ConflictStatus, Operation, SessionRecord, SessionStatus,
    Severity,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use std::str::FromStr;

const NO_LINE: i32 = -1;

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = events)]
pub(super) struct EventRow {
    pub(super) id: i64,
    pub(super) project_id: String,
    pub(super) session_id: Option<String>,
    pub(super) kind: String,
    pub(super) payload_json: String,
    pub(super) created_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = events)]
pub(super) struct NewEvent<'a> {
    pub(super) project_id: &'a str,
    pub(super) session_id: Option<&'a str>,
    pub(super) kind: &'a str,
    pub(super) payload_json: String,
    pub(super) created_at: String,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = sessions)]
pub(super) struct SessionRow {
    pub(super) session_id: String,
    pub(super) project_id: String,
    pub(super) agent: String,
    pub(super) worktree: Option<String>,
    pub(super) task_summary: Option<String>,
    #[allow(dead_code)]
    pub(super) prompt_hash: Option<String>,
    pub(super) status: String,
    #[allow(dead_code)]
    pub(super) config_hash: String,
    #[allow(dead_code)]
    pub(super) started_at: String,
    pub(super) last_seen_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = sessions)]
pub(super) struct NewSession<'a> {
    pub(super) session_id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) agent: &'a str,
    pub(super) worktree: Option<&'a str>,
    pub(super) task_summary: Option<&'a str>,
    pub(super) prompt_hash: Option<&'a str>,
    pub(super) status: &'a str,
    pub(super) config_hash: &'a str,
    pub(super) started_at: &'a str,
    pub(super) last_seen_at: &'a str,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = claims)]
pub(super) struct ClaimRow {
    pub(super) id: String,
    pub(super) project_id: String,
    pub(super) session_id: String,
    pub(super) tool_use_id: String,
    pub(super) path: String,
    pub(super) operation: String,
    pub(super) line_start: i32,
    pub(super) line_end: i32,
    pub(super) state: String,
    pub(super) expires_at: String,
    pub(super) updated_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = claims)]
pub(super) struct NewClaim<'a> {
    pub(super) id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) session_id: &'a str,
    pub(super) tool_use_id: &'a str,
    pub(super) path: &'a str,
    pub(super) operation: &'a str,
    pub(super) line_start: i32,
    pub(super) line_end: i32,
    pub(super) state: &'a str,
    pub(super) expires_at: &'a str,
    pub(super) updated_at: &'a str,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = conflicts)]
pub(super) struct ConflictRow {
    pub(super) id: String,
    pub(super) project_id: String,
    pub(super) left_claim_id: String,
    pub(super) right_claim_id: String,
    pub(super) path: String,
    pub(super) severity: String,
    pub(super) kind: String,
    pub(super) status: String,
    pub(super) message: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = conflicts)]
pub(super) struct NewConflict<'a> {
    pub(super) id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) left_claim_id: &'a str,
    pub(super) right_claim_id: &'a str,
    pub(super) path: &'a str,
    pub(super) severity: &'a str,
    pub(super) kind: &'a str,
    pub(super) status: &'a str,
    pub(super) message: &'a str,
    pub(super) created_at: &'a str,
    pub(super) updated_at: &'a str,
}

#[derive(Insertable)]
#[diesel(table_name = advisories)]
pub(super) struct NewAdvisory<'a> {
    pub(super) id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) session_id: &'a str,
    pub(super) tool_use_id: &'a str,
    pub(super) conflict_id: Option<&'a str>,
    pub(super) severity: &'a str,
    pub(super) kind: &'a str,
    pub(super) path: &'a str,
    pub(super) message: &'a str,
    pub(super) created_at: &'a str,
}

#[derive(Insertable)]
#[diesel(table_name = capability_uses)]
pub(super) struct NewCapabilityUse<'a> {
    pub(super) id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) agent: &'a str,
    pub(super) session_id: &'a str,
    pub(super) turn_id: Option<&'a str>,
    pub(super) invocation_id: &'a str,
    pub(super) parent_invocation_id: Option<&'a str>,
    pub(super) kind: &'a str,
    pub(super) name: &'a str,
    pub(super) source: &'a str,
    pub(super) evidence: &'a str,
    pub(super) outcome: &'a str,
    pub(super) config_hash: &'a str,
    pub(super) model_id: Option<&'a str>,
    pub(super) first_observed_at: &'a str,
    pub(super) completed_at: Option<&'a str>,
}

pub(super) fn claim_from_row(row: ClaimRow) -> Result<Claim> {
    Ok(Claim {
        id: row.id,
        project_id: row.project_id,
        session_id: row.session_id,
        tool_use_id: row.tool_use_id,
        path: row.path,
        operation: Operation::from_str(&row.operation).map_err(anyhow::Error::msg)?,
        line_start: decode_line(row.line_start),
        line_end: decode_line(row.line_end),
        state: ClaimState::from_str(&row.state).map_err(anyhow::Error::msg)?,
        expires_at: parse_time(&row.expires_at)?,
        updated_at: parse_time(&row.updated_at)?,
    })
}

pub(super) fn session_from_row(row: SessionRow) -> Result<SessionRecord> {
    Ok(SessionRecord {
        session_id: row.session_id,
        project_id: row.project_id,
        agent: row.agent,
        worktree: row.worktree,
        status: SessionStatus::from_str(&row.status).map_err(anyhow::Error::msg)?,
        task_summary: row.task_summary,
        last_seen_at: parse_time(&row.last_seen_at)?,
    })
}

pub(super) fn conflict_from_row(row: ConflictRow) -> Result<ConflictRecord> {
    Ok(ConflictRecord {
        id: row.id,
        project_id: row.project_id,
        left_claim_id: row.left_claim_id,
        right_claim_id: row.right_claim_id,
        path: row.path,
        severity: Severity::from_str(&row.severity).map_err(anyhow::Error::msg)?,
        kind: row.kind,
        status: ConflictStatus::from_str(&row.status).map_err(anyhow::Error::msg)?,
        message: row.message,
        created_at: parse_time(&row.created_at)?,
        updated_at: parse_time(&row.updated_at)?,
    })
}

pub(super) fn encode_line(line: Option<u32>) -> i32 {
    line.and_then(|line| i32::try_from(line).ok())
        .unwrap_or(NO_LINE)
}

fn decode_line(line: i32) -> Option<u32> {
    u32::try_from(line).ok()
}

pub(super) fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}
