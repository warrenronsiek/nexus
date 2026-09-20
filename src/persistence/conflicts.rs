// @feature persistence
// @spec docs/features/persistence.md
use super::models::{conflict_from_row, ConflictRow, NewAdvisory, NewConflict};
use super::schema::{advisories, conflicts};
use super::store::{append_event, Store};
use crate::agents::analyst::AnalysisResult;
use crate::coordination::domain::{Advisory, Claim, ConflictRecord, ConflictScope};
use anyhow::Result;
use chrono::Utc;
use diesel::prelude::*;
use serde_json::json;

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
        self.connection
            .transaction::<_, anyhow::Error, _>(|connection| {
                diesel::insert_into(conflicts::table)
                    .values(NewConflict {
                        id: &conflict_id,
                        project_id: project_key,
                        left_claim_id: &left_claim.id,
                        right_claim_id: &right_claim.id,
                        path: &advisory.path,
                        severity: &severity,
                        kind: &advisory.kind,
                        status: "open",
                        message: &advisory.message,
                        created_at: &now,
                        updated_at: &now,
                    })
                    .on_conflict(conflicts::id)
                    .do_update()
                    .set((
                        conflicts::left_claim_id.eq(&left_claim.id),
                        conflicts::right_claim_id.eq(&right_claim.id),
                        conflicts::severity.eq(&severity),
                        conflicts::kind.eq(&advisory.kind),
                        conflicts::status.eq("open"),
                        conflicts::message.eq(&advisory.message),
                        conflicts::updated_at.eq(&now),
                    ))
                    .execute(connection)?;
                diesel::insert_into(advisories::table)
                    .values(NewAdvisory {
                        id: &advisory.id,
                        project_id: project_key,
                        session_id: &left_claim.session_id,
                        tool_use_id: tool_key,
                        conflict_id: Some(&conflict_id),
                        severity: &severity,
                        kind: &advisory.kind,
                        path: &advisory.path,
                        message: &advisory.message,
                        created_at: &now,
                    })
                    .execute(connection)?;
                append_event(
                    connection,
                    project_key,
                    Some(&left_claim.session_id),
                    "conflict_detected",
                    serde_json::to_value(&advisory)?,
                )
            })
            .map(|()| advisory)
    }

    pub fn list_conflicts(
        &mut self,
        project_key: Option<&str>,
        scope: ConflictScope,
    ) -> Result<Vec<ConflictRecord>> {
        let mut query = conflicts::table.into_boxed();
        if let Some(project_key) = project_key {
            query = query.filter(conflicts::project_id.eq(project_key));
        }
        if scope == ConflictScope::Open {
            query = query.filter(conflicts::status.eq("open"));
        }
        let rows = query
            .order(conflicts::created_at.desc())
            .select(ConflictRow::as_select())
            .load::<ConflictRow>(&mut self.connection)?;
        rows.into_iter().map(conflict_from_row).collect()
    }

    pub(crate) fn conflict_by_id(&mut self, conflict_key: &str) -> Result<Option<ConflictRecord>> {
        conflicts::table
            .filter(conflicts::id.eq(conflict_key))
            .select(ConflictRow::as_select())
            .first::<ConflictRow>(&mut self.connection)
            .optional()?
            .map(conflict_from_row)
            .transpose()
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
        self.connection
            .transaction::<_, anyhow::Error, _>(|connection| {
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
