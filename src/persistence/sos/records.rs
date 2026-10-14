// @feature sos
// @feature persistence
// @spec docs/features/sos.md
// @spec docs/features/persistence.md
// @boundary persisted-json
use crate::persistence::models::parse_time;
use crate::persistence::schema::{
    sos_issue_lifecycle, sos_issues, sos_reports, sos_triage_attempts,
};
use crate::sos::{
    SosActor, SosIssue, SosIssueLifecycleEvent, SosIssueStatus, SosIssueTransition, SosMetadata,
    SosReport, SosTriageAttempt, SosTriageDecision, SosTriageOutcome,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use std::str::FromStr;

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = sos_reports)]
pub(super) struct SosReportRow {
    pub(super) id: String,
    pub(super) project_id: String,
    pub(super) issue_id: Option<String>,
    pub(super) message: String,
    pub(super) report_type: String,
    pub(super) environment: String,
    pub(super) version: String,
    pub(super) model: String,
    pub(super) trace: String,
    pub(super) metadata_json: String,
    pub(super) reporter_agent: String,
    pub(super) reporter_session_id: String,
    pub(super) reporter_model_id: Option<String>,
    pub(super) reporter_config_hash: String,
    pub(super) created_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = sos_reports)]
pub(super) struct NewSosReport<'a> {
    pub(super) id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) issue_id: Option<&'a str>,
    pub(super) message: &'a str,
    pub(super) report_type: &'a str,
    pub(super) environment: &'a str,
    pub(super) version: &'a str,
    pub(super) model: &'a str,
    pub(super) trace: &'a str,
    pub(super) metadata_json: &'a str,
    pub(super) reporter_agent: &'a str,
    pub(super) reporter_session_id: &'a str,
    pub(super) reporter_model_id: Option<&'a str>,
    pub(super) reporter_config_hash: &'a str,
    pub(super) created_at: &'a str,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = sos_issues)]
pub(super) struct SosIssueRow {
    pub(super) id: String,
    pub(super) project_id: String,
    pub(super) title: String,
    pub(super) summary: String,
    pub(super) status: String,
    pub(super) resolution: Option<String>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) closed_at: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = sos_issues)]
pub(super) struct NewSosIssue<'a> {
    pub(super) id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) title: &'a str,
    pub(super) summary: &'a str,
    pub(super) status: &'a str,
    pub(super) resolution: Option<&'a str>,
    pub(super) created_at: &'a str,
    pub(super) updated_at: &'a str,
    pub(super) closed_at: Option<&'a str>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = sos_triage_attempts)]
pub(super) struct SosTriageAttemptRow {
    pub(super) id: String,
    pub(super) project_id: String,
    pub(super) report_id: String,
    pub(super) issue_id: Option<String>,
    pub(super) outcome: String,
    pub(super) provider: String,
    pub(super) model_id: Option<String>,
    pub(super) rationale: String,
    pub(super) generated_title: Option<String>,
    pub(super) generated_summary: Option<String>,
    pub(super) config_hash: String,
    pub(super) created_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = sos_triage_attempts)]
pub(super) struct NewSosTriageAttempt<'a> {
    pub(super) id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) report_id: &'a str,
    pub(super) issue_id: Option<&'a str>,
    pub(super) outcome: &'a str,
    pub(super) provider: &'a str,
    pub(super) model_id: Option<&'a str>,
    pub(super) rationale: &'a str,
    pub(super) generated_title: Option<&'a str>,
    pub(super) generated_summary: Option<&'a str>,
    pub(super) config_hash: &'a str,
    pub(super) created_at: &'a str,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = sos_issue_lifecycle)]
pub(super) struct SosLifecycleRow {
    pub(super) id: String,
    pub(super) project_id: String,
    pub(super) issue_id: String,
    pub(super) transition: String,
    pub(super) reason: String,
    pub(super) actor_agent: String,
    pub(super) actor_session_id: String,
    pub(super) actor_model_id: Option<String>,
    pub(super) actor_config_hash: String,
    pub(super) created_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = sos_issue_lifecycle)]
pub(super) struct NewSosLifecycle<'a> {
    pub(super) id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) issue_id: &'a str,
    pub(super) transition: &'a str,
    pub(super) reason: &'a str,
    pub(super) actor_agent: &'a str,
    pub(super) actor_session_id: &'a str,
    pub(super) actor_model_id: Option<&'a str>,
    pub(super) actor_config_hash: &'a str,
    pub(super) created_at: &'a str,
}

pub(crate) struct SosTriageApply<'a> {
    pub project_id: &'a str,
    pub report_id: &'a str,
    pub decision: &'a SosTriageDecision,
    pub provider: &'a str,
    pub model_id: Option<&'a str>,
    pub config_hash: &'a str,
}

pub(crate) struct SosTriageFailure<'a> {
    pub project_id: &'a str,
    pub report_id: &'a str,
    pub provider: &'a str,
    pub model_id: Option<&'a str>,
    pub rationale: &'a str,
    pub config_hash: &'a str,
}

impl SosReportRow {
    pub(super) fn decode(self) -> Result<SosReport> {
        let metadata = SosMetadata::try_from(
            serde_json::from_str::<serde_json::Value>(&self.metadata_json)
                .context("decode SOS metadata JSON")?,
        )?;
        let created_at = parse_time(&self.created_at)?;
        Ok(SosReport {
            id: self.id,
            project_id: self.project_id,
            issue_id: self.issue_id,
            message: self.message,
            kind: self.report_type,
            environment: self.environment,
            version: self.version,
            model: self.model,
            trace: self.trace,
            metadata,
            reporter: decode_actor(
                self.reporter_agent,
                self.reporter_session_id,
                self.reporter_model_id,
                self.reporter_config_hash,
            ),
            created_at,
        })
    }
}

impl SosTriageAttemptRow {
    pub(super) fn decode(self) -> Result<SosTriageAttempt> {
        let outcome = SosTriageOutcome::from_str(&self.outcome).map_err(anyhow::Error::msg)?;
        let created_at = parse_time(&self.created_at)?;
        Ok(SosTriageAttempt {
            id: self.id,
            project_id: self.project_id,
            report_id: self.report_id,
            issue_id: self.issue_id,
            outcome,
            provider: self.provider,
            model_id: self.model_id,
            rationale: self.rationale,
            generated_title: self.generated_title,
            generated_summary: self.generated_summary,
            config_hash: self.config_hash,
            created_at,
        })
    }
}

impl SosLifecycleRow {
    pub(super) fn decode(self) -> Result<SosIssueLifecycleEvent> {
        let transition =
            SosIssueTransition::from_str(&self.transition).map_err(anyhow::Error::msg)?;
        let created_at = parse_time(&self.created_at)?;
        Ok(SosIssueLifecycleEvent {
            id: self.id,
            project_id: self.project_id,
            issue_id: self.issue_id,
            transition,
            reason: self.reason,
            actor: decode_actor(
                self.actor_agent,
                self.actor_session_id,
                self.actor_model_id,
                self.actor_config_hash,
            ),
            created_at,
        })
    }
}

fn decode_actor(
    agent: String,
    session_id: String,
    model_id: Option<String>,
    config_hash: String,
) -> SosActor {
    SosActor {
        agent,
        session_id,
        model_id,
        config_hash,
    }
}

struct IssueTimes {
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    closed_at: Option<DateTime<Utc>>,
    first_reported_at: DateTime<Utc>,
    last_reported_at: DateTime<Utc>,
}

impl IssueTimes {
    fn decode(row: &SosIssueRow, first: &str, last: &str) -> Result<Self> {
        Ok(Self {
            created_at: parse_time(&row.created_at)?,
            updated_at: parse_time(&row.updated_at)?,
            closed_at: row.closed_at.as_deref().map(parse_time).transpose()?,
            first_reported_at: parse_time(first)?,
            last_reported_at: parse_time(last)?,
        })
    }
}

pub(super) fn decode_issue(
    row: SosIssueRow,
    first_reported_at: String,
    last_reported_at: String,
    importance_count: i64,
) -> Result<SosIssue> {
    let status = SosIssueStatus::from_str(&row.status).map_err(anyhow::Error::msg)?;
    let times = IssueTimes::decode(&row, &first_reported_at, &last_reported_at)?;
    let importance_count =
        u64::try_from(importance_count).context("negative SOS importance count")?;
    Ok(SosIssue {
        id: row.id,
        project_id: row.project_id,
        title: row.title,
        summary: row.summary,
        status,
        resolution: row.resolution,
        created_at: times.created_at,
        updated_at: times.updated_at,
        closed_at: times.closed_at,
        first_reported_at: times.first_reported_at,
        last_reported_at: times.last_reported_at,
        importance_count,
    })
}
