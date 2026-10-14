// @feature sos
// @feature persistence
// @spec docs/features/sos.md
// @spec docs/features/persistence.md
use super::records::{NewSosReport, NewSosTriageAttempt, SosTriageApply, SosTriageFailure};
use crate::persistence::schema::{sos_reports, sos_triage_attempts};
use crate::persistence::test_support::actor;
use crate::persistence::Store;
use crate::sos::{
    SosIssueStatus, SosMetadata, SosReportDraft, SosTriageDecision, SosTriageOutcome,
};
use chrono::{TimeZone, Utc};
use diesel::prelude::*;
use serde_json::json;

fn draft(message: &str) -> SosReportDraft {
    SosReportDraft {
        message: message.into(),
        kind: "build_failure".into(),
        environment: "ci".into(),
        version: "1.0.0".into(),
        model: "gpt-test".into(),
        trace: "test suite timed out".into(),
        metadata: SosMetadata::try_from(json!({"job":"validate"})).unwrap(),
    }
}

#[test]
fn triage_links_every_source_and_reopens_a_resolved_aggregate() {
    let mut store = Store::open_memory().unwrap();
    let first = store
        .record_sos_report("project-a", draft("Validation hangs"), &actor("reporter-a"))
        .unwrap();
    let create = SosTriageDecision::Create {
        title: "Validation jobs hang".into(),
        summary: "Validation jobs stop making progress and eventually time out.".into(),
        rationale: "No existing aggregate describes the failure.".into(),
    };
    let created = store
        .apply_sos_triage(SosTriageApply {
            project_id: "project-a",
            report_id: &first.id,
            decision: &create,
            provider: "codex",
            model_id: Some("triage-model"),
            config_hash: "config-a",
        })
        .unwrap();

    assert_eq!(created.status, SosIssueStatus::Open);
    assert_eq!(created.importance_count, 1);
    assert_eq!(created.first_reported_at, first.created_at);
    assert_eq!(created.last_reported_at, first.created_at);

    let resolved = store
        .resolve_sos_issue(
            "project-a",
            &created.id,
            "Made the watchdog terminate the stalled subprocess.",
            &actor("resolver"),
        )
        .unwrap();
    assert_eq!(resolved.status, SosIssueStatus::Resolved);

    let second = store
        .record_sos_report(
            "project-a",
            draft("The validation command is hanging again"),
            &actor("reporter-b"),
        )
        .unwrap();
    let link = SosTriageDecision::Link {
        issue_id: created.id.clone(),
        title: "Validation jobs hang repeatedly".into(),
        summary: "Validation jobs intermittently stop making progress despite the watchdog fix."
            .into(),
        rationale: "The new report is a recurrence of the same stalled validation job.".into(),
    };
    let reopened = store
        .apply_sos_triage(SosTriageApply {
            project_id: "project-a",
            report_id: &second.id,
            decision: &link,
            provider: "codex",
            model_id: Some("triage-model"),
            config_hash: "config-a",
        })
        .unwrap();

    assert_eq!(reopened.status, SosIssueStatus::Open);
    assert_eq!(reopened.importance_count, 2);
    assert_eq!(reopened.first_reported_at, first.created_at);
    assert_eq!(reopened.last_reported_at, second.created_at);
    assert_eq!(reopened.resolution, None);
    let detail = store.sos_issue_detail("project-a", &created.id).unwrap();
    assert_eq!(detail.reports.len(), 2);
    assert_eq!(detail.triage_attempts.len(), 2);
    assert_eq!(detail.triage_attempts[0].outcome, SosTriageOutcome::Created);
    assert_eq!(detail.triage_attempts[1].outcome, SosTriageOutcome::Linked);
    assert_eq!(detail.lifecycle_events.len(), 2);
    assert_eq!(
        detail.lifecycle_events[0].transition.to_string(),
        "resolved"
    );
    assert_eq!(
        detail.lifecycle_events[1].transition.to_string(),
        "reopened"
    );
}

#[test]
fn aggregate_links_and_reads_are_project_isolated() {
    let mut store = Store::open_memory().unwrap();
    let project_a_report = store
        .record_sos_report(
            "project-a",
            draft("Project A failure"),
            &actor("reporter-a"),
        )
        .unwrap();
    let create = SosTriageDecision::Create {
        title: "Project A build failure".into(),
        summary: "Only project A has reported this failure.".into(),
        rationale: "No aggregate exists yet.".into(),
    };
    let project_a_issue = store
        .apply_sos_triage(SosTriageApply {
            project_id: "project-a",
            report_id: &project_a_report.id,
            decision: &create,
            provider: "codex",
            model_id: Some("triage-model"),
            config_hash: "config-a",
        })
        .unwrap();
    let project_b_report = store
        .record_sos_report(
            "project-b",
            draft("Project B failure"),
            &actor("reporter-b"),
        )
        .unwrap();
    let cross_project_link = SosTriageDecision::Link {
        issue_id: project_a_issue.id.clone(),
        title: "Cross-project link".into(),
        summary: "This must not be persisted.".into(),
        rationale: "The identifier came from another project.".into(),
    };

    assert!(store
        .apply_sos_triage(SosTriageApply {
            project_id: "project-b",
            report_id: &project_b_report.id,
            decision: &cross_project_link,
            provider: "codex",
            model_id: Some("triage-model"),
            config_hash: "config-a",
        })
        .is_err());
    assert!(store
        .sos_issue_detail("project-b", &project_a_issue.id)
        .is_err());
    let project_b = store.open_sos_issues_window("project-b").unwrap();
    assert!(project_b.issues.is_empty());
    assert_eq!(project_b.untriaged_report_count, 1);
}

#[test]
fn failed_triage_retry_cooldown_grows_exponentially_and_caps_at_one_day() {
    use super::projections::sos_retry_cooldown;
    use chrono::Duration;

    assert_eq!(sos_retry_cooldown(0), Duration::zero());
    assert_eq!(sos_retry_cooldown(1), Duration::minutes(5));
    assert_eq!(sos_retry_cooldown(2), Duration::minutes(10));
    assert_eq!(sos_retry_cooldown(5), Duration::minutes(80));
    assert_eq!(sos_retry_cooldown(64), Duration::hours(24));
}

#[test]
fn failed_untriaged_reports_wait_for_their_persisted_retry_cooldown() {
    let mut store = Store::open_memory().unwrap();
    let report = store
        .record_sos_report("project-a", draft("Build is stuck"), &actor("reporter"))
        .unwrap();
    assert_eq!(store.ready_sos_reports().unwrap().len(), 1);

    store
        .record_sos_triage_failure(SosTriageFailure {
            project_id: "project-a",
            report_id: &report.id,
            provider: "codex",
            model_id: Some("triage-model"),
            rationale: "provider execution is disabled",
            config_hash: "config-a",
        })
        .unwrap();
    assert!(store.ready_sos_reports().unwrap().is_empty());

    let retryable_at = (Utc::now() - chrono::Duration::minutes(6)).to_rfc3339();
    diesel::update(
        sos_triage_attempts::table.filter(sos_triage_attempts::report_id.eq(&report.id)),
    )
    .set(sos_triage_attempts::created_at.eq(retryable_at))
    .execute(&mut store.connection)
    .unwrap();
    assert_eq!(store.ready_sos_reports().unwrap().len(), 1);
}

#[test]
fn recovery_rotates_past_a_cooling_backlog_without_unbounded_loading() {
    let mut store = Store::open_memory().unwrap();
    let created_at = "2026-10-14T12:00:00+00:00";
    for ordinal in 0..129 {
        let report_id = format!("recovery-report-{ordinal:03}");
        diesel::insert_into(sos_reports::table)
            .values(NewSosReport {
                id: &report_id,
                project_id: "project-a",
                issue_id: None,
                message: "Provider unavailable",
                report_type: "provider_failure",
                environment: "ci",
                version: "1.0.0",
                model: "gpt-test",
                trace: "timeout",
                metadata_json: "{}",
                reporter_agent: "codex",
                reporter_session_id: "recovery-session",
                reporter_model_id: Some("gpt-test"),
                reporter_config_hash: "config-a",
                created_at,
            })
            .execute(&mut store.connection)
            .unwrap();
        if ordinal < 128 {
            let attempt_id = format!("recovery-attempt-{ordinal:03}");
            diesel::insert_into(sos_triage_attempts::table)
                .values(NewSosTriageAttempt {
                    id: &attempt_id,
                    project_id: "project-a",
                    report_id: &report_id,
                    issue_id: None,
                    outcome: "failed",
                    provider: "codex",
                    model_id: Some("triage-model"),
                    rationale: "Provider unavailable",
                    generated_title: None,
                    generated_summary: None,
                    config_hash: "config-a",
                    created_at,
                })
                .execute(&mut store.connection)
                .unwrap();
        }
    }

    let second_page_tick = Utc.timestamp_opt(60, 0).unwrap();
    let ready = store.ready_sos_reports_at(second_page_tick).unwrap();

    assert_eq!(
        ready,
        vec![("project-a".into(), "recovery-report-128".into())]
    );
}

#[test]
fn issue_detail_stays_bounded_beyond_sqlite_variable_limits() {
    let mut store = Store::open_memory().unwrap();
    let first = store
        .record_sos_report("project-a", draft("First failure"), &actor("reporter"))
        .unwrap();
    let create = SosTriageDecision::Create {
        title: "Repeated build failure".into(),
        summary: "Build workers repeatedly fail.".into(),
        rationale: "First report creates the aggregate.".into(),
    };
    let issue = store
        .apply_sos_triage(SosTriageApply {
            project_id: "project-a",
            report_id: &first.id,
            decision: &create,
            provider: "codex",
            model_id: Some("triage-model"),
            config_hash: "config-a",
        })
        .unwrap();
    let now = Utc::now().to_rfc3339();
    for ordinal in 0..1_005 {
        let report_id = format!("bulk-report-{ordinal:04}");
        diesel::insert_into(sos_reports::table)
            .values(NewSosReport {
                id: &report_id,
                project_id: "project-a",
                issue_id: Some(&issue.id),
                message: "Repeated failure",
                report_type: "build_failure",
                environment: "ci",
                version: "1.0.0",
                model: "gpt-test",
                trace: "timeout",
                metadata_json: "{}",
                reporter_agent: "codex",
                reporter_session_id: "bulk-session",
                reporter_model_id: Some("gpt-test"),
                reporter_config_hash: "config-a",
                created_at: &now,
            })
            .execute(&mut store.connection)
            .unwrap();
        let attempt_id = format!("bulk-attempt-{ordinal:04}");
        diesel::insert_into(sos_triage_attempts::table)
            .values(NewSosTriageAttempt {
                id: &attempt_id,
                project_id: "project-a",
                report_id: &report_id,
                issue_id: Some(&issue.id),
                outcome: "linked",
                provider: "codex",
                model_id: Some("triage-model"),
                rationale: "Same issue",
                generated_title: Some("Repeated build failure"),
                generated_summary: Some("Build workers repeatedly fail."),
                config_hash: "config-a",
                created_at: &now,
            })
            .execute(&mut store.connection)
            .unwrap();
    }

    let detail = store.sos_issue_detail("project-a", &issue.id).unwrap();

    assert_eq!(
        detail.reports.len(),
        super::projections::MAX_SOS_DETAIL_REPORTS
    );
    assert!(detail.reports_truncated);
    assert_eq!(
        detail.triage_attempts.len(),
        super::projections::MAX_SOS_DETAIL_ATTEMPTS
    );
    assert!(detail.triage_attempts_truncated);
}

#[test]
fn issue_detail_is_bounded_after_json_escaping() {
    let mut store = Store::open_memory().unwrap();
    let first = store
        .record_sos_report("project-a", draft("First failure"), &actor("reporter"))
        .unwrap();
    let create = SosTriageDecision::Create {
        title: "Repeated build failure".into(),
        summary: "Build workers repeatedly fail.".into(),
        rationale: "First report creates the aggregate.".into(),
    };
    let issue = store
        .apply_sos_triage(SosTriageApply {
            project_id: "project-a",
            report_id: &first.id,
            decision: &create,
            provider: "codex",
            model_id: Some("triage-model"),
            config_hash: "config-a",
        })
        .unwrap();
    let now = Utc::now().to_rfc3339();
    let escaped_trace = "\0".repeat(64 * 1024);
    for ordinal in 0..3 {
        let report_id = format!("escaped-report-{ordinal}");
        diesel::insert_into(sos_reports::table)
            .values(NewSosReport {
                id: &report_id,
                project_id: "project-a",
                issue_id: Some(&issue.id),
                message: "Failure with a control-heavy trace",
                report_type: "build_failure",
                environment: "ci",
                version: "1.0.0",
                model: "gpt-test",
                trace: &escaped_trace,
                metadata_json: "{}",
                reporter_agent: "codex",
                reporter_session_id: "escaped-session",
                reporter_model_id: Some("gpt-test"),
                reporter_config_hash: "config-a",
                created_at: &now,
            })
            .execute(&mut store.connection)
            .unwrap();
    }

    let detail = store.sos_issue_detail("project-a", &issue.id).unwrap();
    let encoded = serde_json::to_vec(&detail).unwrap();

    assert!(encoded.len() <= super::projections::MAX_SOS_DETAIL_SERIALIZED_BYTES);
    assert!(detail.reports_truncated);
    assert_eq!(detail.reports.last().unwrap().id, "escaped-report-2");
}
