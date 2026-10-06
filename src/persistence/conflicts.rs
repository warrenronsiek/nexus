// @feature persistence
// @spec docs/features/persistence.md
use super::models::{conflict_from_row, ConflictRow, NewAdvisory, NewConflict};
use super::schema::{advisories, conflicts};
use super::store::{append_event, Store};
use crate::agents::analyst::AnalysisResult;
use crate::coordination::domain::{Advisory, Claim, ConflictRecord, ConflictScope, ConflictStatus};
use anyhow::Result;
use chrono::Utc;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde_json::json;

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
        find_conflict(&mut self.connection, conflict_key)
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
    connection: &mut SqliteConnection,
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
    if let Some(previous) =
        current.filter(|previous| observation.advisory.severity > previous.severity)
    {
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
        if previous.status == ConflictStatus::Resolved {
            append_event(
                connection,
                observation.project_key,
                Some(&observation.left_claim.session_id),
                "conflict_reopened",
                json!({
                    "conflict_id": observation.conflict_id,
                    "previous_severity": previous.severity,
                    "new_severity": observation.advisory.severity,
                }),
            )?;
        }
    }
    Ok(())
}

fn record_occurrence(
    connection: &mut SqliteConnection,
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
    connection: &mut SqliteConnection,
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
