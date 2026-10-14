// @feature sos
// @feature persistence
// @spec docs/features/sos.md
// @spec docs/features/persistence.md
use super::lifecycle::{insert_lifecycle, LifecycleWrite};
use super::mutations::load_report;
use super::projections::load_issue;
use super::records::{NewSosIssue, NewSosTriageAttempt, SosTriageApply, SosTriageFailure};
use crate::persistence::backend::DbConnection;
use crate::persistence::schema::{sos_issues, sos_reports, sos_triage_attempts};
use crate::persistence::store::append_event;
use crate::persistence::transaction::{
    prepared_immediate_transaction, transaction, TransactionMode,
};
use crate::persistence::Store;
use crate::sos::{
    required_text, SosActor, SosIssue, SosIssueStatus, SosIssueTransition, SosTriageDecision,
    MAX_SOS_IDENTITY_BYTES, MAX_SOS_TEXT_BYTES, MAX_SOS_TITLE_BYTES,
};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use diesel::prelude::*;
use serde_json::json;
use uuid::Uuid;

struct PreparedTriage<'a> {
    input: SosTriageApply<'a>,
    provenance: TriageProvenance,
    generated: GeneratedIssueText,
}

struct TriageProvenance {
    provider: String,
    model_id: Option<String>,
    config_hash: String,
}

impl TriageProvenance {
    fn new(provider: &str, model_id: Option<&str>, config_hash: &str) -> Result<Self> {
        Ok(Self {
            provider: required_text("SOS triage provider", provider, MAX_SOS_IDENTITY_BYTES)?,
            model_id: model_id
                .map(|value| required_text("SOS triage model", value, MAX_SOS_IDENTITY_BYTES))
                .transpose()?,
            config_hash: required_text(
                "SOS triage config hash",
                config_hash,
                MAX_SOS_IDENTITY_BYTES,
            )?,
        })
    }
}

struct GeneratedIssueText {
    title: String,
    summary: String,
    rationale: String,
}

impl GeneratedIssueText {
    fn new(decision: &SosTriageDecision) -> Result<Self> {
        let (title, summary, rationale) = match decision {
            SosTriageDecision::Create {
                title,
                summary,
                rationale,
            }
            | SosTriageDecision::Link {
                title,
                summary,
                rationale,
                ..
            } => (title, summary, rationale),
        };
        Ok(Self {
            title: required_text("SOS issue title", title, MAX_SOS_TITLE_BYTES)?,
            summary: required_text("SOS issue summary", summary, MAX_SOS_TEXT_BYTES)?,
            rationale: required_text("SOS triage rationale", rationale, MAX_SOS_TEXT_BYTES)?,
        })
    }
}

impl<'a> PreparedTriage<'a> {
    fn new(input: SosTriageApply<'a>) -> Result<Self> {
        let provenance = TriageProvenance::new(input.provider, input.model_id, input.config_hash)?;
        let generated = GeneratedIssueText::new(input.decision)?;
        Ok(Self {
            input,
            provenance,
            generated,
        })
    }
}

struct PreparedFailure<'a> {
    input: SosTriageFailure<'a>,
    provenance: TriageProvenance,
    rationale: String,
    created_at: String,
}

impl<'a> PreparedFailure<'a> {
    fn new(input: SosTriageFailure<'a>) -> Result<Self> {
        Ok(Self {
            provenance: TriageProvenance::new(input.provider, input.model_id, input.config_hash)?,
            rationale: required_text("SOS triage failure", input.rationale, MAX_SOS_TEXT_BYTES)?,
            input,
            created_at: Utc::now().to_rfc3339(),
        })
    }
}

impl Store {
    pub(crate) fn apply_sos_triage(&mut self, input: SosTriageApply<'_>) -> Result<SosIssue> {
        let write = PreparedTriage::new(input)?;
        let issue_id = transaction(
            &mut self.connection,
            TransactionMode::Immediate,
            |connection| apply_transaction(connection, &write),
        )?;
        load_issue(&mut self.connection, write.input.project_id, &issue_id)
    }

    pub(crate) fn record_sos_triage_failure(
        &mut self,
        failure: SosTriageFailure<'_>,
    ) -> Result<()> {
        prepared_immediate_transaction(
            &mut self.connection,
            failure,
            PreparedFailure::new,
            record_failure_transaction,
        )
    }
}

struct AppliedIssue {
    id: String,
    outcome: &'static str,
}

fn apply_transaction(connection: &mut DbConnection, write: &PreparedTriage<'_>) -> Result<String> {
    let report = load_report(connection, write.input.project_id, write.input.report_id)?;
    if let Some(issue_id) = report.issue_id {
        return Ok(issue_id);
    }
    let now = Utc::now().to_rfc3339();
    let applied = apply_decision(connection, write, &report, &now)?;
    link_report(connection, write, &applied.id)?;
    record_success(connection, write, &report, &applied, &now)?;
    Ok(applied.id)
}

fn apply_decision(
    connection: &mut DbConnection,
    write: &PreparedTriage<'_>,
    report: &crate::sos::SosReport,
    now: &str,
) -> Result<AppliedIssue> {
    match write.input.decision {
        SosTriageDecision::Create { .. } => create_issue(connection, write, now),
        SosTriageDecision::Link { issue_id, .. } => {
            link_issue(connection, write, report, issue_id, now)
        }
    }
}

fn create_issue(
    connection: &mut DbConnection,
    write: &PreparedTriage<'_>,
    now: &str,
) -> Result<AppliedIssue> {
    let issue_id = Uuid::new_v4().to_string();
    diesel::insert_into(sos_issues::table)
        .values(NewSosIssue {
            id: &issue_id,
            project_id: write.input.project_id,
            title: &write.generated.title,
            summary: &write.generated.summary,
            status: "open",
            resolution: None,
            created_at: now,
            updated_at: now,
            closed_at: None,
        })
        .execute(connection)?;
    Ok(AppliedIssue {
        id: issue_id,
        outcome: "created",
    })
}

fn link_issue(
    connection: &mut DbConnection,
    write: &PreparedTriage<'_>,
    report: &crate::sos::SosReport,
    issue_id: &str,
    now: &str,
) -> Result<AppliedIssue> {
    let issue = load_issue(connection, write.input.project_id, issue_id)?;
    let actor = triage_actor(write, &report.id);
    if issue.status != SosIssueStatus::Open {
        insert_lifecycle(
            connection,
            LifecycleWrite {
                project_id: write.input.project_id,
                issue_id,
                transition: SosIssueTransition::Reopened,
                reason: "A new SOS report showed that the issue is active again.",
                actor: &actor,
                created_at: now,
            },
        )?;
    }
    diesel::update(
        sos_issues::table
            .filter(sos_issues::project_id.eq(write.input.project_id))
            .find(issue_id),
    )
    .set((
        sos_issues::title.eq(&write.generated.title),
        sos_issues::summary.eq(&write.generated.summary),
        sos_issues::status.eq("open"),
        sos_issues::resolution.eq::<Option<String>>(None),
        sos_issues::updated_at.eq(now),
        sos_issues::closed_at.eq::<Option<String>>(None),
    ))
    .execute(connection)?;
    Ok(AppliedIssue {
        id: issue_id.to_owned(),
        outcome: "linked",
    })
}

fn link_report(
    connection: &mut DbConnection,
    write: &PreparedTriage<'_>,
    issue_id: &str,
) -> Result<()> {
    let linked = diesel::update(
        sos_reports::table
            .filter(sos_reports::project_id.eq(write.input.project_id))
            .filter(sos_reports::id.eq(write.input.report_id))
            .filter(sos_reports::issue_id.is_null()),
    )
    .set(sos_reports::issue_id.eq(Some(issue_id)))
    .execute(connection)?;
    if linked != 1 {
        bail!("SOS report {} could not be linked", write.input.report_id);
    }
    Ok(())
}

fn record_success(
    connection: &mut DbConnection,
    write: &PreparedTriage<'_>,
    report: &crate::sos::SosReport,
    issue: &AppliedIssue,
    now: &str,
) -> Result<()> {
    insert_attempt(
        connection,
        AttemptWrite {
            project_id: write.input.project_id,
            report_id: write.input.report_id,
            issue_id: Some(&issue.id),
            outcome: issue.outcome,
            provenance: &write.provenance,
            rationale: &write.generated.rationale,
            generated_title: Some(&write.generated.title),
            generated_summary: Some(&write.generated.summary),
            created_at: now,
        },
    )?;
    append_event(
        connection,
        write.input.project_id,
        Some(&report.reporter.session_id),
        "sos_triaged",
        json!({
            "report_id": write.input.report_id,
            "issue_id": issue.id,
            "outcome": issue.outcome,
        }),
    )?;
    Ok(())
}

struct AttemptWrite<'a> {
    project_id: &'a str,
    report_id: &'a str,
    issue_id: Option<&'a str>,
    outcome: &'a str,
    provenance: &'a TriageProvenance,
    rationale: &'a str,
    generated_title: Option<&'a str>,
    generated_summary: Option<&'a str>,
    created_at: &'a str,
}

fn insert_attempt(connection: &mut DbConnection, write: AttemptWrite<'_>) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    diesel::insert_into(sos_triage_attempts::table)
        .values(NewSosTriageAttempt {
            id: &id,
            project_id: write.project_id,
            report_id: write.report_id,
            issue_id: write.issue_id,
            outcome: write.outcome,
            provider: &write.provenance.provider,
            model_id: write.provenance.model_id.as_deref(),
            rationale: write.rationale,
            generated_title: write.generated_title,
            generated_summary: write.generated_summary,
            config_hash: &write.provenance.config_hash,
            created_at: write.created_at,
        })
        .execute(connection)
        .context("record SOS triage attempt")?;
    Ok(())
}

fn triage_actor(write: &PreparedTriage<'_>, report_id: &str) -> SosActor {
    SosActor {
        agent: format!("nexus-{}-sos-triage", write.provenance.provider),
        session_id: format!("sos-triage-{report_id}"),
        model_id: write.provenance.model_id.clone(),
        config_hash: write.provenance.config_hash.clone(),
    }
}

fn record_failure_transaction(
    connection: &mut DbConnection,
    write: &PreparedFailure<'_>,
) -> Result<()> {
    let report = load_report(connection, write.input.project_id, write.input.report_id)?;
    if report.issue_id.is_some() {
        return Ok(());
    }
    insert_attempt(
        connection,
        AttemptWrite {
            project_id: write.input.project_id,
            report_id: write.input.report_id,
            issue_id: None,
            outcome: "failed",
            provenance: &write.provenance,
            rationale: &write.rationale,
            generated_title: None,
            generated_summary: None,
            created_at: &write.created_at,
        },
    )?;
    append_event(
        connection,
        write.input.project_id,
        Some(&report.reporter.session_id),
        "sos_triage_failed",
        json!({"report_id": write.input.report_id}),
    )?;
    Ok(())
}
