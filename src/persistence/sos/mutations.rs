// @feature sos
// @feature persistence
// @spec docs/features/sos.md
// @spec docs/features/persistence.md
// @entrypoint Store::record_sos_report
use super::records::{NewSosReport, SosReportRow};
use crate::config::canonical_project_id;
use crate::persistence::backend::DbConnection;
use crate::persistence::schema::sos_reports;
use crate::persistence::store::append_event;
use crate::persistence::transaction::{transaction, TransactionMode};
use crate::persistence::Store;
use crate::sos::{validate_actor, SosActor, SosReport, SosReportDraft};
use anyhow::{Context, Result};
use chrono::Utc;
use diesel::prelude::*;
use serde_json::json;
use uuid::Uuid;

struct PreparedReport<'a> {
    project_id: &'a str,
    draft: SosReportDraft,
    actor: &'a SosActor,
    metadata_json: String,
    id: String,
    created_at: String,
}

impl<'a> PreparedReport<'a> {
    fn new(project_id: &'a str, draft: SosReportDraft, actor: &'a SosActor) -> Result<Self> {
        let project_id = canonical_project_id(project_id)
            .map_err(|_| anyhow::anyhow!("SOS project_id must contain 1 to 256 bytes"))?;
        validate_actor(actor)?;
        let draft = draft.normalized()?;
        let metadata_json =
            serde_json::to_string(&draft.metadata).context("encode SOS metadata")?;
        Ok(Self {
            project_id,
            draft,
            actor,
            metadata_json,
            id: Uuid::new_v4().to_string(),
            created_at: Utc::now().to_rfc3339(),
        })
    }

    fn insert(&self, connection: &mut DbConnection) -> Result<()> {
        diesel::insert_into(sos_reports::table)
            .values(NewSosReport {
                id: &self.id,
                project_id: self.project_id,
                issue_id: None,
                message: &self.draft.message,
                report_type: &self.draft.kind,
                environment: &self.draft.environment,
                version: &self.draft.version,
                model: &self.draft.model,
                trace: &self.draft.trace,
                metadata_json: &self.metadata_json,
                reporter_agent: &self.actor.agent,
                reporter_session_id: &self.actor.session_id,
                reporter_model_id: self.actor.model_id.as_deref(),
                reporter_config_hash: &self.actor.config_hash,
                created_at: &self.created_at,
            })
            .execute(connection)?;
        append_event(
            connection,
            self.project_id,
            Some(&self.actor.session_id),
            "sos_reported",
            json!({"report_id": self.id}),
        )?;
        Ok(())
    }
}

impl Store {
    pub fn record_sos_report(
        &mut self,
        project_id: &str,
        draft: SosReportDraft,
        actor: &SosActor,
    ) -> Result<SosReport> {
        let report = PreparedReport::new(project_id, draft, actor)?;
        transaction(
            &mut self.connection,
            TransactionMode::Immediate,
            |connection| report.insert(connection),
        )?;
        load_report(&mut self.connection, report.project_id, &report.id)
    }

    pub(crate) fn sos_report(&mut self, project_id: &str, report_id: &str) -> Result<SosReport> {
        load_report(&mut self.connection, project_id, report_id)
    }
}

pub(super) fn load_report(
    connection: &mut DbConnection,
    project_id: &str,
    report_id: &str,
) -> Result<SosReport> {
    let row = sos_reports::table
        .filter(sos_reports::project_id.eq(project_id))
        .find(report_id)
        .select(SosReportRow::as_select())
        .first::<SosReportRow>(connection)
        .optional()?
        .with_context(|| format!("unknown SOS report {report_id}"))?;
    row.decode()
}
