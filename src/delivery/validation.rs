// @feature delivery-coordination
// @spec docs/features/delivery-coordination.md
use super::*;
use anyhow::{bail, Result};

pub(crate) fn text(value: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max || value.contains('\0') {
        bail!("invalid bounded delivery text");
    }
    Ok(())
}

fn text_fields(fields: &[(&str, usize)]) -> Result<()> {
    fields.iter().try_for_each(|(value, max)| text(value, *max))
}

pub(crate) fn git(value: &crate::work_history::GitRevision) -> Result<()> {
    text(&value.repository, 512)?;
    let size = match value.object_format.as_str() {
        "sha1" => 40,
        "sha256" => 64,
        _ => bail!("invalid Git object format"),
    };
    if value.commit.len() != size || !value.commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("full Git revision required");
    }
    Ok(())
}

fn path(value: &str) -> Result<()> {
    text(value, 4096)?;
    if std::path::Path::new(value).is_absolute() || value.split('/').any(|part| part == "..") {
        bail!("repository-relative path required");
    }
    Ok(())
}

impl AgentIdentity {
    pub fn validate(&self) -> Result<()> {
        text(&self.agent, 256)?;
        text(&self.session_id, 512)
    }
}
impl DeliveryObservationInput {
    pub fn validate(&self) -> Result<()> {
        text(&self.id, 512)?;
        if serde_json::to_vec(self)?.len() > 64 * 1024 {
            bail!("delivery observation exceeds 64 KiB");
        }
        match &self.body {
            DeliveryEvent::EnvironmentInventoryObserved { inventory } => {
                inventory.validate(self.occurred_at)?
            }
            DeliveryEvent::EnvironmentAdvisoryReported { advisory } => {
                advisory.validate(self.occurred_at)?
            }
            _ => {}
        }
        self.body.validate()
    }
}
impl DeliverySubject {
    pub fn validate(&self) -> Result<()> {
        text(&self.id, 512)
    }
}
impl DeliveryEvent {
    pub fn subject(&self) -> Option<DeliverySubject> {
        let (kind, id) = match self {
            Self::ContributionRegistered { contribution } => {
                (DeliveryKind::Contribution, &contribution.id)
            }
            Self::RevisionMapped { mapping } => {
                (DeliveryKind::Contribution, &mapping.contribution_id)
            }
            Self::PullRequestRegistered { pull_request } => {
                (DeliveryKind::PullRequest, &pull_request.id)
            }
            Self::PullRequestObserved {
                pull_request_id, ..
            }
            | Self::PullRequestCompositionRecorded {
                pull_request_id, ..
            } => (DeliveryKind::PullRequest, pull_request_id),
            Self::RolloutRegistered { rollout } => (DeliveryKind::Rollout, &rollout.id),
            Self::DeploymentRecorded { deployment } => (DeliveryKind::Deployment, &deployment.id),
            Self::DeploymentFinished { deployment_id, .. } => {
                (DeliveryKind::Deployment, deployment_id)
            }
            Self::ShepherdAssigned { subject, .. } => return Some(subject.clone()),
            Self::ReleaseRequirementsRecorded { requirements } => {
                (DeliveryKind::ReleaseRequirements, &requirements.id)
            }
            Self::EnvironmentInventoryObserved { inventory } => {
                (DeliveryKind::EnvironmentInventory, &inventory.id)
            }
            Self::EnvironmentAdvisoryReported { advisory } => {
                (DeliveryKind::EnvironmentAdvisory, &advisory.id)
            }
            Self::RemediationRequested { remediation } => {
                (DeliveryKind::Remediation, &remediation.id)
            }
            Self::RemediationTransitioned { remediation_id, .. } => {
                (DeliveryKind::Remediation, remediation_id)
            }
            _ => return None,
        };
        Some(DeliverySubject {
            kind,
            id: id.clone(),
        })
    }

    fn validate(&self) -> Result<()> {
        self.subject()
            .as_ref()
            .map(DeliverySubject::validate)
            .transpose()?;
        match self {
            Self::ContributionRegistered { contribution } => contribution.validate(),
            Self::RevisionMapped { mapping } => mapping.validate(),
            Self::PullRequestRegistered { pull_request } => pull_request.validate(),
            Self::PullRequestObserved { snapshot, .. } => snapshot.validate(),
            Self::PullRequestCompositionRecorded {
                head,
                contribution_ids,
                reason,
                ..
            } => validate_composition(head, contribution_ids, reason),
            Self::RolloutRegistered { rollout } => rollout.validate(),
            Self::DeploymentRecorded { deployment } => deployment.validate(),

            Self::ShepherdAssigned {
                subject,
                agent,
                reason,
                ..
            } => validate_shepherd(subject, agent.as_ref(), reason),
            Self::AgentPresenceObserved { presence } => presence.agent.validate(),
            Self::RemediationRequested { remediation } => remediation.validate(),
            Self::RemediationTransitioned {
                reason,
                fixing_revision,
                ..
            } => {
                text(reason, 2048)?;
                fixing_revision.as_ref().map(git).transpose().map(|_| ())
            }
            _ => self.validate_context_event(),
        }
    }

    fn validate_context_event(&self) -> Result<()> {
        match self {
            Self::ReleaseRequirementsRecorded { requirements } => requirements.validate(),
            Self::EnvironmentInventoryObserved { .. }
            | Self::EnvironmentAdvisoryReported { .. } => Ok(()),
            Self::WatchRegistered { watch } => watch.validate(),
            Self::WatchStopped { watch_id } => text(watch_id, 512),
            Self::DeploymentFinished { .. } => Ok(()),
            _ => unreachable!("core delivery event"),
        }
    }
}
impl Contribution {
    fn validate(&self) -> Result<()> {
        text_fields(&[
            (&self.repository, 512),
            (&self.title, 1024),
            (&self.rationale, 8192),
        ])?;
        self.origin.validate()?;
        for value in &self.constraints {
            text(value, 2048)?;
        }
        for value in &self.features {
            text(value, 256)?;
        }
        for revision in [&self.base_revision, &self.head_revision]
            .into_iter()
            .flatten()
        {
            git(revision)?;
            if revision.repository != self.repository {
                bail!("contribution revision repository mismatch");
            }
        }
        if self.features.len() > 64 || self.constraints.len() > 32 {
            bail!("too many contribution labels");
        }
        Ok(())
    }
}
impl RevisionMapping {
    fn validate(&self) -> Result<()> {
        text(&self.id, 512)?;
        git(&self.revision)?;
        if self.hunks.len() > 256 || self.source_revisions.len() > 64 {
            bail!("mapping exceeds bounded evidence");
        }
        for revision in &self.source_revisions {
            git(revision)?;
            if revision.repository != self.revision.repository {
                bail!("cross-repository mapping");
            }
        }
        for hunk in &self.hunks {
            hunk.validate()?;
        }
        Ok(())
    }
}
impl ChangedHunk {
    fn validate(&self) -> Result<()> {
        path(&self.path)?;
        if let Some(value) = &self.fingerprint {
            fingerprint(value)?;
        }
        if let Some(old) = &self.old_path {
            path(old)?;
        }
        if self.new_start.checked_add(self.new_lines).is_none()
            || self.old_start.checked_add(self.old_lines).is_none()
        {
            bail!("hunk range overflow");
        }
        Ok(())
    }
}
impl PullRequest {
    fn validate(&self) -> Result<()> {
        text_fields(&[
            (&self.repository, 512),
            (&self.title, 1024),
            (&self.provider, 128),
            (&self.url, 4096),
        ])?;
        if self.number == 0 || self.contribution_ids.len() > 256 {
            bail!("invalid pull request");
        }
        git(&self.base)?;
        git(&self.head)?;
        if self.base.repository != self.repository || self.head.repository != self.repository {
            bail!("pull request repository mismatch");
        }
        Ok(())
    }
}
impl PullRequestSnapshot {
    fn validate(&self) -> Result<()> {
        git(&self.base)?;
        git(&self.head)?;
        if self.checks.len() > 128 {
            bail!("too many checks");
        }
        for check in &self.checks {
            text(&check.id, 512)?;
            text(&check.name, 1024)?;
            git(&check.revision)?;
        }
        Ok(())
    }
}
impl Rollout {
    fn validate(&self) -> Result<()> {
        text(&self.repository, 512)?;
        text(&self.title, 1024)?;
        if self.contribution_ids.len() > 256
            || self.pull_request_ids.len() > 256
            || self.environments.len() > 64
        {
            bail!("rollout exceeds bound");
        }
        for environment in &self.environments {
            text(environment, 256)?;
        }
        Ok(())
    }
}
impl Deployment {
    fn validate(&self) -> Result<()> {
        text(&self.repository, 512)?;
        text(&self.rollout_id, 512)?;
        text(&self.environment, 256)?;
        git(&self.revision)?;
        if self.attempt == 0 || self.revision.repository != self.repository {
            bail!("invalid deployment attempt");
        }
        Ok(())
    }
}
fn validate_shepherd(
    subject: &DeliverySubject,
    agent: Option<&AgentIdentity>,
    reason: &str,
) -> Result<()> {
    if !matches!(
        subject.kind,
        DeliveryKind::PullRequest | DeliveryKind::Rollout
    ) {
        bail!("shepherd requires a pull request or rollout");
    }
    if let Some(agent) = agent {
        agent.validate()?;
    }
    text(reason, 2048)
}
impl Remediation {
    fn validate(&self) -> Result<()> {
        text_fields(&[(&self.repository, 512), (&self.title, 1024)])?;
        self.recipient.validate()?;
        text(&self.request, 8192)?;
        if self.evidence.len() > 32 {
            bail!("too many remediation evidence links");
        }
        for item in &self.evidence {
            text(item, 4096)?;
        }
        if let Some(revision) = &self.failure_revision {
            git(revision)?;
        }
        Ok(())
    }
}
impl OriginQuery {
    pub fn validate(&self) -> Result<()> {
        self.validate_coordinates()?;
        self.validate_fingerprint()?;
        self.validate_selectors()?;
        if self.line_start == Some(0)
            || self.line_end == Some(0)
            || matches!((self.line_start, self.line_end), (Some(a),Some(b)) if b < a)
        {
            bail!("invalid origin line range");
        }
        if self.repository.is_none() && !self.has_selector() {
            bail!("origin lookup requires a repository or an origin selector");
        }
        Ok(())
    }
    pub(crate) fn has_selector(&self) -> bool {
        [
            self.revision.is_some(),
            self.revision_prefix.is_some(),
            self.fingerprint.is_some(),
            self.pull_request_id.is_some(),
            self.pull_request_number.is_some(),
            self.rollout_id.is_some(),
            self.branch.is_some(),
            self.feature.is_some(),
        ]
        .into_iter()
        .any(|present| present)
    }

    fn validate_selectors(&self) -> Result<()> {
        if let Some(prefix) = &self.revision_prefix {
            if self.repository.is_none()
                || self.revision.is_some()
                || !(4..=64).contains(&prefix.len())
                || !prefix.bytes().all(|b| b.is_ascii_hexdigit())
            {
                bail!("revision prefix requires a repository and 4 to 64 hexadecimal characters");
            }
        }
        if self.pull_request_number.is_some_and(|number| number == 0)
            || (self.pull_request_number.is_some()
                && (self.repository.is_none() || self.pull_request_id.is_some()))
            || (self.pull_request_provider.is_some() && self.pull_request_number.is_none())
        {
            bail!(
                "provider PR number requires a repository and cannot be combined with a record ID"
            );
        }
        for selector in [&self.branch, &self.pull_request_provider]
            .into_iter()
            .flatten()
        {
            text(selector, 512)?;
        }
        Ok(())
    }

    fn validate_fingerprint(&self) -> Result<()> {
        if let Some(value) = &self.fingerprint {
            fingerprint(value)?;
            if self.revision.is_none() && self.repository.is_none() {
                bail!("fingerprint lookup requires repository identity")
            }
        }
        if self.line_end.is_some() && self.line_start.is_none() {
            bail!("line_end requires line_start")
        }
        Ok(())
    }
    fn validate_coordinates(&self) -> Result<()> {
        if let Some(revision) = &self.revision {
            git(revision)?;
        }
        if let Some(value) = &self.path {
            path(value)?;
        }
        if self.revision.is_none()
            && self.revision_prefix.is_none()
            && self.fingerprint.is_none()
            && (self.path.is_some() || self.line_start.is_some() || self.line_end.is_some())
        {
            bail!("path and range origin queries require an immutable revision");
        }
        if let (Some(revision), Some(repository)) = (&self.revision, &self.repository) {
            if &revision.repository != repository {
                bail!("origin repository identity conflicts with revision");
            }
        }
        if self.line_start.is_some() && self.path.is_none() {
            bail!("origin line range requires a path");
        }
        Ok(())
    }
}

fn fingerprint(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("fingerprint requires a full content digest")
    }
    Ok(())
}

fn validate_composition(
    head: &crate::work_history::GitRevision,
    contributions: &[String],
    reason: &str,
) -> Result<()> {
    git(head)?;
    text(reason, 2048)?;
    if contributions.len() > 256 {
        bail!("pull request composition exceeds 256 contributions")
    }
    for id in contributions {
        text(id, 512)?;
    }
    Ok(())
}
