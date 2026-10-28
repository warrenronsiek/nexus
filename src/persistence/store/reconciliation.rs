// @feature persistence
// @feature coordination
// @spec docs/features/persistence.md
// @spec docs/features/coordination.md
use super::{
    active_session_exists, append_event,
    claim_writes::{insert_claim_in_transaction, ClaimOrigin},
    Store,
};
use crate::coordination::domain::PathIntent;
use crate::persistence::{
    backend::DbConnection,
    models::{encode_line, parse_time, ClaimRow},
    schema::claims,
};
use anyhow::Result;
use chrono::{Duration, Utc};
use diesel::prelude::*;
use serde_json::json;

impl Store {
    pub(crate) fn record_reconciliation(
        &mut self,
        project_key: &str,
        session_key: &str,
        intents: &[PathIntent],
        ttl_seconds: i64,
    ) -> Result<()> {
        self.transaction(|connection| {
            if !active_session_exists(connection, project_key, session_key)? {
                return Ok(());
            }
            let mut newly_observed = Vec::new();
            for intent in intents {
                if refresh_reconciled_claim(
                    connection,
                    project_key,
                    session_key,
                    intent,
                    ttl_seconds,
                )? {
                    newly_observed.push(intent);
                }
            }
            if !newly_observed.is_empty() {
                append_event(
                    connection,
                    project_key,
                    Some(session_key),
                    "git_reconciled",
                    json!({"changed_paths":newly_observed}),
                )?;
            }
            Ok(())
        })
    }
}

fn refresh_reconciled_claim(
    connection: &mut DbConnection,
    project_key: &str,
    session_key: &str,
    intent: &PathIntent,
    ttl_seconds: i64,
) -> Result<bool> {
    let tool_key = format!(
        "git-reconcile:{}",
        blake3::hash(intent.path.as_bytes()).to_hex()
    );
    let existing = reconciled_claim(connection, project_key, session_key, intent, &tool_key)?;
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
            .execute(connection)?;
        return Ok(false);
    }
    insert_claim_in_transaction(
        connection,
        &ClaimOrigin {
            project_id: project_key,
            session_id: session_key,
            tool_use_id: &tool_key,
        },
        intent,
        ttl_seconds,
    )?;
    Ok(true)
}

fn reconciled_claim(
    connection: &mut DbConnection,
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
        .first::<ClaimRow>(connection)
        .optional()?)
}
