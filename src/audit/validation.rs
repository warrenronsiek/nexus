// @feature audit-registry
// @spec docs/features/audit-registry.md
use super::*;
use anyhow::{ensure, Result};
use std::collections::HashSet;

pub(crate) fn identity(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 160
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:@/".contains(&byte)),
        "invalid audit identifier"
    );
    Ok(())
}

fn text(value: &str, limit: usize) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= limit && !value.contains('\0'),
        "empty, oversized or invalid audit text"
    );
    Ok(())
}

impl ControlVersion {
    pub(crate) fn validate(&self) -> Result<()> {
        identity(&self.key)?;
        ensure!(self.version > 0, "control version must be positive");
        Ok(())
    }
}

impl RunFilter {
    pub(crate) fn validate(&self) -> Result<()> {
        self.control_key.as_deref().map(identity).transpose()?;
        ensure!(self.run_ids.len() <= 64, "select at most 64 audit run IDs");
        self.run_ids.iter().try_for_each(|id| identity(id))?;
        ensure!(
            self.version
                .is_none_or(|version| version > 0 && self.control_key.is_some()),
            "version requires a control key and positive version"
        );
        ensure!(
            self.from
                .zip(self.until)
                .is_none_or(|(from, until)| from < until),
            "audit period must have from before until"
        );
        Ok(())
    }
}

impl AuditInput {
    pub(crate) fn validate(&self) -> Result<()> {
        identity(&self.id)?;
        self.event.validate_fields()?;
        self.event
            .reference_ids()
            .into_iter()
            .flatten()
            .try_for_each(identity)?;
        self.event.evidence_lists().try_for_each(|items| {
            ensure!(items.len() <= 32, "too many evidence references");
            items.iter().try_for_each(AuditEvidence::validate)
        })
    }
}

impl AuditEvent {
    fn reference_ids(&self) -> [Option<&str>; 3] {
        match self {
            Self::Define { previous, .. } => [previous.as_deref(), None, None],
            Self::Retire { previous, .. } => [Some(previous), None, None],
            Self::Run { run, .. } => [run.retry_of.as_deref(), None, None],
            Self::CorrectRun {
                run_id, previous, ..
            } => [Some(run_id), Some(previous), None],
            Self::Disposition {
                run_id,
                finding_id,
                previous,
                ..
            } => [Some(run_id), finding_id.as_deref(), previous.as_deref()],
            Self::Verification {
                run_id,
                disposition_id,
                ..
            } => [Some(run_id), Some(disposition_id), None],
        }
    }

    fn validate_fields(&self) -> Result<()> {
        match self {
            Self::Define {
                control,
                definition,
                ..
            } => control.validate().and_then(|()| definition.validate()),
            Self::Retire {
                control, reason, ..
            } => control.validate().and_then(|()| text(reason, 8192)),
            Self::Run { control, run } => control.validate().and_then(|()| run.validate()),
            Self::CorrectRun { reason, result, .. } => {
                text(reason, 8192).and_then(|()| result.validate())
            }
            Self::Disposition { reason, .. } => text(reason, 8192),
            Self::Verification {
                verifier,
                procedure,
                reason,
                ..
            } => text(verifier, 512).and_then(|()| {
                [procedure, reason]
                    .into_iter()
                    .try_for_each(|value| text(value, 8192))
            }),
        }
    }
}

impl AuditRun {
    fn validate(&self) -> Result<()> {
        self.subject.validate()?;
        text(&self.executor, 512)?;
        self.subject.validate_for_status(self.result.status)?;
        self.result.validate()
    }
}

fn categories(items: &[Category]) -> Result<()> {
    ensure!(items.len() <= 64, "too many categories");
    let mut seen = HashSet::new();
    items.iter().try_for_each(|item| {
        identity(&item.code)?;
        text(&item.label, 256)?;
        ensure!(seen.insert(&item.code), "duplicate category code");
        Ok(())
    })
}

impl ControlDefinition {
    fn validate(&self) -> Result<()> {
        text(&self.owner, 512)?;
        for field in [
            &self.objective,
            &self.scope,
            &self.trigger,
            &self.procedure,
            &self.reason,
        ] {
            text(field, 8192)?;
        }
        if let ResultSchema::Categorical { codes } = &self.results {
            ensure!(
                !codes.is_empty(),
                "categorical controls require result codes"
            );
            categories(codes)?;
        }
        categories(&self.finding_categories)?;
        ensure!(
            self.framework_references.len() <= 32,
            "too many framework references"
        );
        self.framework_references
            .iter()
            .try_for_each(|value| text(value, 512))
    }

    pub(crate) fn validate_result(&self, result: &RunResult) -> Result<()> {
        match (&self.results, &result.output) {
            (_, None) if result.status != RunStatus::Completed => {}
            (ResultSchema::Narrative, Some(RunOutput::Narrative(_))) => {}
            (ResultSchema::Categorical { codes }, Some(RunOutput::Code(value))) => {
                ensure!(
                    codes.iter().any(|code| code.code == *value),
                    "unknown result code"
                );
            }
            _ => anyhow::bail!("run output does not match the control version"),
        }
        for category in result
            .findings
            .iter()
            .filter_map(|finding| finding.category.as_ref())
        {
            ensure!(
                self.finding_categories
                    .iter()
                    .any(|item| item.code == *category),
                "unknown finding category"
            );
        }
        Ok(())
    }
}

impl AuditSubject {
    pub(crate) fn validate_for_status(&self, status: RunStatus) -> Result<()> {
        ensure!(
            !matches!(self, Self::StagedDiff { sha256: None, .. })
                || matches!(status, RunStatus::Failed | RunStatus::Skipped),
            "a performed staged review requires the diff digest"
        );
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        match self {
            Self::External { source, id } => {
                text(source, 256)?;
                text(id, 2048)
            }
            Self::Git { revision } => validate_git(revision),
            Self::StagedDiff {
                repository,
                base,
                sha256,
            } => {
                text(repository, 4096)?;
                if let Some(base) = base {
                    ensure!(
                        base.repository == *repository,
                        "staged base is from a different repository"
                    );
                    validate_git(base)?;
                }
                ensure!(
                    sha256.as_deref().is_none_or(crate::evals::digest_is_valid),
                    "invalid staged diff digest"
                );
                Ok(())
            }
        }
    }
}

fn validate_git(revision: &GitRevision) -> Result<()> {
    text(&revision.repository, 4096)?;
    let length = match revision.object_format.as_str() {
        "sha1" => 40,
        "sha256" => 64,
        _ => anyhow::bail!("invalid Git object format"),
    };
    ensure!(
        revision.commit.len() == length && revision.commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "Git revision requires a full object ID"
    );
    Ok(())
}

impl RunResult {
    fn validate(&self) -> Result<()> {
        self.validate_output()?;
        ensure!(
            self.findings.len() <= 64 && self.review_passes.len() <= 16,
            "too many findings or review passes"
        );
        let mut ids = HashSet::new();
        self.findings.iter().try_for_each(|finding| {
            ensure!(ids.insert(&finding.id), "duplicate finding id");
            finding.validate()
        })?;
        self.review_passes.iter().try_for_each(ReviewPass::validate)
    }

    fn validate_output(&self) -> Result<()> {
        ensure!(
            self.status == RunStatus::Completed || self.summary.is_some(),
            "an incomplete run requires a reason"
        );
        self.summary
            .as_deref()
            .map(|value| text(value, 8192))
            .transpose()?;
        match &self.output {
            Some(RunOutput::Narrative(value)) => text(value, 8192),
            Some(RunOutput::Code(value)) => identity(value),
            None => Ok(()),
        }
    }
}

impl Finding {
    fn validate(&self) -> Result<()> {
        identity(&self.id)?;
        text(&self.description, 4096)?;
        self.category.as_deref().map(identity).transpose()?;
        self.recommendation
            .as_deref()
            .map(|value| text(value, 4096))
            .transpose()?;
        Ok(())
    }
}

impl ReviewPass {
    fn validate(&self) -> Result<()> {
        text(&self.perspective, 256)?;
        text(&self.provider, 256)?;
        self.model
            .as_deref()
            .map(|value| text(value, 512))
            .transpose()?;
        self.summary
            .as_deref()
            .map(|value| text(value, 8192))
            .transpose()?;
        ensure!(
            self.instructions_sha256
                .as_deref()
                .is_none_or(crate::evals::digest_is_valid),
            "invalid review instructions digest"
        );
        Ok(())
    }
}

impl AuditEvidence {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Git {
                revision,
                description,
            } => {
                validate_git(revision)?;
                text(description, 2048)
            }
            Self::Work {
                subject,
                description,
            } => {
                text(subject.id(), 512)?;
                text(description, 2048)
            }
            Self::Delivery {
                subject,
                description,
            } => {
                text(&subject.id, 512)?;
                text(description, 2048)
            }
            Self::StoredBlob {
                digest,
                description,
            } => {
                ensure!(
                    crate::evals::digest_is_valid(digest),
                    "invalid evidence digest"
                );
                text(description, 2048)
            }
            Self::External { uri, description } => {
                text(uri, 4096)?;
                text(description, 2048)
            }
            Self::Unavailable { reason } => text(reason, 2048),
        }
    }
}
