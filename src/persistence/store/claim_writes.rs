// @feature persistence
// @feature coordination
// @spec docs/features/persistence.md
// @spec docs/features/coordination.md
use super::{append_event, Store};
use crate::coordination::domain::{Claim, ClaimState, PathIntent};
use crate::persistence::{
    backend::DbConnection,
    models::{encode_line, NewClaim},
    schema::claims,
};
use anyhow::Result;
use chrono::{Duration, Utc};
use diesel::prelude::*;
use serde_json::json;
use uuid::Uuid;

impl Store {
    pub(crate) fn insert_claim(
        &mut self,
        project_key: &str,
        session_key: &str,
        tool_key: &str,
        intent: &PathIntent,
        ttl_seconds: i64,
    ) -> Result<Claim> {
        self.transaction(|connection| {
            insert_claim_in_transaction(
                connection,
                &ClaimOrigin {
                    project_id: project_key,
                    session_id: session_key,
                    tool_use_id: tool_key,
                },
                intent,
                ttl_seconds,
            )
        })
    }
}

pub(super) struct ClaimOrigin<'a> {
    pub(super) project_id: &'a str,
    pub(super) session_id: &'a str,
    pub(super) tool_use_id: &'a str,
}

pub(super) fn insert_claim_in_transaction(
    connection: &mut DbConnection,
    origin: &ClaimOrigin<'_>,
    intent: &PathIntent,
    ttl_seconds: i64,
) -> Result<Claim> {
    let ClaimOrigin {
        project_id: project_key,
        session_id: session_key,
        tool_use_id: tool_key,
    } = *origin;
    let new_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let now_text = now.to_rfc3339();
    let expires = now + Duration::seconds(ttl_seconds);
    let expires_text = expires.to_rfc3339();
    let operation_text = intent.operation.to_string();
    let start = encode_line(intent.line_start);
    let end = encode_line(intent.line_end);
    let id = {
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
        let stored_id = upsert_claim_row(connection, &new_claim)?;
        append_event(
            connection,
            project_key,
            Some(session_key),
            "claim_created",
            json!({"claim_id":stored_id,"tool_use_id":tool_key,"path":intent.path,"operation":intent.operation,"line_start":intent.line_start,"line_end":intent.line_end,"expires_at":expires}),
        )?;
        stored_id
    };
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

fn upsert_claim_row(connection: &mut DbConnection, new_claim: &NewClaim<'_>) -> Result<String> {
    diesel::insert_into(claims::table)
        .values(new_claim)
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
            claims::expires_at.eq(new_claim.expires_at),
            claims::updated_at.eq(new_claim.updated_at),
        ))
        .execute(connection)?;
    Ok(claims::table
        .filter(claims::project_id.eq(new_claim.project_id))
        .filter(claims::session_id.eq(new_claim.session_id))
        .filter(claims::tool_use_id.eq(new_claim.tool_use_id))
        .filter(claims::path.eq(new_claim.path))
        .filter(claims::operation.eq(new_claim.operation))
        .filter(claims::line_start.eq(new_claim.line_start))
        .filter(claims::line_end.eq(new_claim.line_end))
        .select(claims::id)
        .first::<String>(connection)?)
}
