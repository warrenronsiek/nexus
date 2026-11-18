// @feature audit-registry
// @feature commit-review
// @spec docs/features/audit-registry.md
// @spec docs/features/commit-review.md
// @boundary review-json: model output is untrusted text decoded into a closed review report.
use super::*;
use crate::{
    audit::{
        AuditEvent, AuditEvidence, AuditInput, AuditRun, AuditSubject, ControlVersion, Finding,
        ReviewPass, ReviewPassOutcome, RunOutput, RunResult, RunStatus,
    },
    runtime::work_capture::git_revision_context,
    work_history::WorkSubject,
    LoadedConfig,
};
use anyhow::ensure;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Instant;

pub struct AuditedReview {
    pub report: Result<CommitReviewReport>,
    pub record: AuditInput,
}

/// Record the bytes passed to both reviewers, preserving the existing advisory result.
pub fn review_staged_commit_audited(
    loaded: &LoadedConfig,
    root: &Path,
    control: ControlVersion,
) -> AuditedReview {
    let started_at = Utc::now();
    let timer = Instant::now();
    let git = git_revision_context(root).unwrap_or_default();
    let report = review_staged_commit_with_capture(
        loaded,
        root,
        ReviewMode::Audited {
            base: git.revision.as_ref().map(|r| r.commit.as_str()),
        },
    );
    let subject = AuditSubject::StagedDiff {
        repository: git
            .repository
            .unwrap_or_else(|| format!("local:{}", sha256(root.to_string_lossy().as_bytes()))),
        base: git.revision,
        sha256: report
            .as_ref()
            .ok()
            .and_then(|r| r.staged_diff_sha256.clone()),
    };
    let result = result(report.as_ref().ok());
    let duration_ms = u64::try_from(timer.elapsed().as_millis()).ok();
    let executor = format!(
        "Nexus cross-model commit review (implementation provider {})",
        model::provider_name(loaded.config.review.implementation_provider)
    );
    AuditedReview {
        report,
        record: AuditInput {
            id: uuid::Uuid::new_v4().to_string(),
            occurred_at: started_at,
            event: AuditEvent::Run {
                control,
                run: AuditRun {
                    subject,
                    executor,
                    duration_ms,
                    retry_of: None,
                    result,
                },
            },
        },
    }
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn result(report: Option<&CommitReviewReport>) -> RunResult {
    let evidence = report
        .and_then(|report| report.activity_id.as_ref())
        .map(|id| AuditEvidence::Work {
            subject: WorkSubject::Activity { id: id.clone() },
            description: "Review group in Nexus Work history".into(),
        })
        .into_iter()
        .collect();
    let (status, code, summary, passes, findings) = match report {
        Some(report) => match report.status {
            ReviewStatus::Reviewed => summarize(&report.reviews),
            ReviewStatus::Disabled => skipped("Commit review was disabled"),
            ReviewStatus::NoStagedChanges => skipped("The staged diff was empty"),
        },
        None => (
            RunStatus::Failed,
            "inconclusive",
            "The staged diff could not be read",
            Vec::new(),
            Vec::new(),
        ),
    };
    RunResult {
        status,
        output: Some(RunOutput::Code(code.into())),
        summary: Some(summary.into()),
        findings,
        evidence,
        review_passes: passes,
    }
}

type Summary = (
    RunStatus,
    &'static str,
    &'static str,
    Vec<ReviewPass>,
    Vec<Finding>,
);

fn skipped(reason: &'static str) -> Summary {
    (
        RunStatus::Skipped,
        "inconclusive",
        reason,
        Vec::new(),
        Vec::new(),
    )
}

fn summarize(outcomes: &[ReviewOutcome]) -> Summary {
    let (passes, findings): (Vec<_>, Vec<_>) = outcomes.iter().map(pass).unzip();
    let findings: Vec<_> = findings.into_iter().flatten().collect();
    let good = passes
        .iter()
        .filter(|pass| {
            matches!(
                pass.outcome,
                ReviewPassOutcome::Pass | ReviewPassOutcome::ChangesRequested
            )
        })
        .count();
    match (good, findings.is_empty()) {
        (2, true) => (
            RunStatus::Completed,
            "pass",
            "Both review passes reported PASS",
            passes,
            findings,
        ),
        (2, false) => (
            RunStatus::Completed,
            "changes_requested",
            "Reviewers requested changes",
            passes,
            findings,
        ),
        (0, _) => (
            RunStatus::Failed,
            "inconclusive",
            "Neither review produced a valid result",
            passes,
            findings,
        ),
        _ => (
            RunStatus::Partial,
            "inconclusive",
            "One review did not produce a valid result",
            passes,
            findings,
        ),
    }
}

fn pass(review: &ReviewOutcome) -> (ReviewPass, Vec<Finding>) {
    let perspective = match review.perspective {
        ReviewPerspective::Architecture => "architecture",
        ReviewPerspective::Deletion => "deletion",
    };
    let (outcome, summary, findings) =
        match (review.error.as_ref(), review.text.as_deref().map(str::trim)) {
            (Some(_), _) => (
                ReviewPassOutcome::ExecutorError,
                Some("Review provider failed".into()),
                Vec::new(),
            ),
            (None, Some("PASS")) => (ReviewPassOutcome::Pass, None, Vec::new()),
            (None, text) => match text.and_then(|value| findings(perspective, value).ok()) {
                Some((summary, findings)) => {
                    (ReviewPassOutcome::ChangesRequested, Some(summary), findings)
                }
                None => (
                    ReviewPassOutcome::InvalidOutput,
                    Some("Reviewer output did not match the review contract".into()),
                    Vec::new(),
                ),
            },
        };
    (
        ReviewPass {
            perspective: perspective.into(),
            provider: model::provider_name(review.provider).into(),
            model: review.model.clone(),
            instructions_sha256: review.instructions_sha256.clone(),
            outcome,
            summary,
        },
        findings,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestedChanges {
    verdict: ChangesRequested,
    summary: String,
    findings: Vec<ReviewFinding>,
}

#[derive(Deserialize)]
enum ChangesRequested {
    #[serde(rename = "CHANGES_REQUESTED")]
    Valid,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ReviewMeasure {
    Number(f64),
    Text(String),
}

impl std::fmt::Display for ReviewMeasure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(number) => number.fmt(formatter),
            Self::Text(text) => text.fmt(formatter),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewFinding {
    confidence: ReviewMeasure,
    file: String,
    line: Option<ReviewMeasure>,
    symbol: Option<String>,
    evidence: String,
    change: String,
    verification: String,
    estimated_removed_loc: Option<u64>,
    added_loc: Option<u64>,
    net_delta: Option<i64>,
}

fn findings(perspective: &str, source: &str) -> Result<(String, Vec<Finding>)> {
    ensure!(
        source.len() <= 16 * 1024,
        "review pass exceeds audit evidence limit"
    );
    let RequestedChanges {
        verdict: ChangesRequested::Valid,
        summary,
        findings,
    } = serde_json::from_str(source)?;
    ensure!(
        !findings.is_empty() && findings.len() <= 32,
        "invalid review findings count"
    );
    ensure!(valid_text(&summary, 4096), "invalid review summary");
    let findings = findings
        .into_iter()
        .enumerate()
        .map(|(index, finding)| finding.into_audit(perspective, index))
        .collect::<Result<_>>()?;
    ensure!(
        serde_json::to_vec(&findings)?.len() + serde_json::to_vec(&summary)?.len() <= 19 * 1024,
        "structured findings exceed the audit pass limit"
    );
    Ok((summary, findings))
}

impl ReviewFinding {
    fn into_audit(self, perspective: &str, index: usize) -> Result<Finding> {
        let loc = self.loc_estimate(perspective)?;
        let locator = self
            .line
            .as_ref()
            .map(ToString::to_string)
            .or(self.symbol)
            .context("missing finding location")?;
        let confidence = self.confidence.to_string();
        ensure!(
            [
                &self.file,
                &locator,
                &self.evidence,
                &self.change,
                &self.verification,
                &confidence
            ]
            .iter()
            .all(|field| valid_text(field, 4096)),
            "incomplete review finding"
        );
        let description = format!(
            "{}:{}\n{}\nConfidence: {}{}",
            self.file, locator, self.evidence, confidence, loc
        );
        let recommendation = format!("{}\nVerify: {}", self.change, self.verification);
        ensure!(
            valid_text(&description, 4096) && valid_text(&recommendation, 4096),
            "review finding exceeds audit record limits"
        );
        Ok(Finding {
            id: format!("{perspective}-{}", index + 1),
            description,
            category: None,
            recommendation: Some(recommendation),
            evidence: Vec::new(),
        })
    }

    fn loc_estimate(&self, perspective: &str) -> Result<String> {
        let loc = match (self.estimated_removed_loc, self.added_loc, self.net_delta) {
            (Some(removed), Some(added), Some(delta)) => {
                ensure!(
                    i128::from(added) - i128::from(removed) == i128::from(delta),
                    "inconsistent LOC estimate"
                );
                format!("\nLOC estimate: {removed} removed, {added} added, net {delta}")
            }
            _ => String::new(),
        };
        ensure!(
            perspective != "deletion"
                || (!loc.is_empty() && self.net_delta.is_some_and(|delta| delta < 0)),
            "deletion finding requires a negative LOC estimate"
        );
        Ok(loc)
    }
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains('\0')
}
