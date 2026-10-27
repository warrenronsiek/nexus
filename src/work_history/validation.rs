// @feature work-history
// @spec docs/features/work-history.md
use super::*;
use anyhow::{bail, Result};

fn text(name: &str, value: &str, max: usize) -> Result<()> {
    if value.is_empty() || value.len() > max || value.contains('\0') {
        bail!("invalid work {name}");
    }
    Ok(())
}
fn texts(values: impl IntoIterator<Item = impl AsRef<str>>, name: &str, max: usize) -> Result<()> {
    values
        .into_iter()
        .try_for_each(|value| text(name, value.as_ref(), max))
}
fn git(repository: &str, format: &str, commit: &str) -> Result<()> {
    text("repository", repository, 4096)?;
    let length = match format {
        "sha1" => 40,
        "sha256" => 64,
        _ => bail!("invalid work Git object format"),
    };
    if commit.len() != length || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("work Git revision must be a full object ID");
    }
    Ok(())
}
impl ExternalIdentity {
    fn validate(&self) -> Result<()> {
        texts([&self.provider, &self.kind], "external provider/type", 256)?;
        text("external identity", &self.id, 1024)?;
        self.url
            .iter()
            .try_for_each(|url| text("external URL", url, 4096))
    }
}
impl WorkContext {
    fn validate(&self) -> Result<()> {
        texts(
            [
                &self.host_session_id,
                &self.turn_id,
                &self.agent_instance_id,
                &self.model_id,
                &self.repository,
                &self.worktree,
                &self.environment,
            ]
            .into_iter()
            .flatten(),
            "context",
            4096,
        )?;
        self.validate_labels()?;
        self.external
            .iter()
            .try_for_each(ExternalIdentity::validate)?;
        self.revision.iter().try_for_each(|revision| {
            git(
                &revision.repository,
                &revision.object_format,
                &revision.commit,
            )
        })
    }
    fn validate_labels(&self) -> Result<()> {
        if self.external.len() > 32 || self.labels.len() > 32 {
            bail!("too many work context labels");
        }
        self.labels.iter().try_for_each(|(key, value)| {
            text("label", key, 128)?;
            text("label value", value, 1024)
        })
    }
}
impl WorkActor {
    pub(crate) fn validate(&self) -> Result<()> {
        text("agent", &self.agent, 256)?;
        texts([&self.session_id, &self.config_hash], "actor identity", 512)?;
        self.model_id
            .iter()
            .try_for_each(|model| text("model", model, 512))
    }
}
impl WorkObservation {
    pub fn validate(&self) -> Result<()> {
        self.input.validate()?;
        text("source project", &self.project_id, 512)?;
        self.provenance.validate()?;
        self.source.iter().try_for_each(WorkSource::validate)
    }
}
impl WorkSource {
    fn validate(&self) -> Result<()> {
        text("original source project", &self.project_id, 512)?;
        self.provenance.validate()
    }
}
impl Elapsed {
    fn validate(&self) -> Result<()> {
        if self.nanoseconds.is_empty()
            || self.nanoseconds.len() > 80
            || !self.nanoseconds.bytes().all(|byte| byte.is_ascii_digit())
        {
            bail!("work elapsed nanoseconds require exact nonnegative integer text");
        }
        text("timing method", &self.method, 512)?;
        self.clock_id
            .iter()
            .try_for_each(|id| text("clock ID", id, 512))
    }
}
impl WorkSourceClock {
    fn validate(&self) -> Result<()> {
        text("source clock ID", &self.clock_id, 512)?;
        if self.nanoseconds.is_empty()
            || self.nanoseconds.len() > 39
            || !self.nanoseconds.bytes().all(|byte| byte.is_ascii_digit())
            || self.nanoseconds.parse::<u128>().is_err()
        {
            bail!("source clock nanoseconds require a bounded nonnegative integer");
        }
        Ok(())
    }
}
impl WorkObservationInput {
    pub fn validate(&self) -> Result<()> {
        self.validate_envelope()?;
        self.source_clock
            .iter()
            .try_for_each(WorkSourceClock::validate)?;
        self.body.validate(&self.subject)
    }
    fn validate_envelope(&self) -> Result<()> {
        texts(
            [
                &self.id,
                self.subject.id(),
                &self.producer.name,
                &self.producer.version,
                &self.producer.instance_id,
                &self.method,
            ],
            "observation identity",
            512,
        )?;
        if serde_json::to_vec(self)?.len() > 16 * 1024 {
            bail!("work observation exceeds 16 KiB");
        }
        self.correction
            .iter()
            .try_for_each(|correction| correction.validate(&self.id))
    }
}
impl WorkCorrection {
    fn validate(&self, id: &str) -> Result<()> {
        if self.replaces == id {
            bail!("work correction cannot replace itself");
        }
        text("corrected event", &self.replaces, 512)?;
        text("correction reason", &self.reason, 2048)
    }
}
impl WorkEvent {
    fn validate(&self, subject: &WorkSubject) -> Result<()> {
        self.context()
            .into_iter()
            .try_for_each(WorkContext::validate)?;
        match self {
            Self::SessionOpened { title, .. } => session(subject, title),
            Self::ActivityDeclared {
                name,
                kind,
                work_session_id,
                parent_activity_id,
                initiator,
                executor,
                ..
            } => activity(
                subject,
                name,
                kind,
                &[work_session_id, parent_activity_id, initiator, executor],
            ),
            Self::Finished {
                elapsed,
                assessment,
                ..
            } => finished(elapsed.as_ref(), assessment.as_deref()),
            Self::Milestone { name, .. } => text("milestone", name, 1024),
            Self::Linked { relation, target } => validate_link(subject, *relation, target),
            Self::Queued | Self::Started => Ok(()),
        }?;
        self.references()
            .into_iter()
            .try_for_each(|(relation, target)| validate_link(subject, relation, &target))
    }
}
fn session(subject: &WorkSubject, title: &str) -> Result<()> {
    if !matches!(subject, WorkSubject::Session { .. }) {
        bail!("session_opened requires a session subject");
    }
    text("title", title, 1024)
}
fn activity(
    subject: &WorkSubject,
    name: &str,
    kind: &str,
    identities: &[&Option<String>],
) -> Result<()> {
    if !matches!(subject, WorkSubject::Activity { .. }) {
        bail!("activity_declared requires an activity subject");
    }
    text("activity name", name, 1024)?;
    text("activity kind", kind, 256)?;
    texts(
        identities.iter().filter_map(|value| value.as_deref()),
        "activity identity",
        512,
    )
}
fn finished(elapsed: Option<&Elapsed>, assessment: Option<&str>) -> Result<()> {
    elapsed.into_iter().try_for_each(Elapsed::validate)?;
    assessment
        .into_iter()
        .try_for_each(|value| text("assessment", value, 2048))
}
impl WorkReference {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Session { id } | Self::Activity { id } | Self::Eval { id } => {
                text("linked subject", id, 512)
            }
            Self::Revision {
                repository,
                object_format,
                commit,
            } => git(repository, object_format, commit),
            Self::External {
                provider,
                kind,
                id,
                url,
            } => ExternalIdentity {
                provider: provider.clone(),
                kind: kind.clone(),
                id: id.clone(),
                url: url.clone(),
            }
            .validate(),
        }
    }
}
fn validate_link(
    source: &WorkSubject,
    relation: WorkRelation,
    target: &WorkReference,
) -> Result<()> {
    target.validate()?;
    if target.subject().as_ref() == Some(source) {
        bail!("work relationship cannot reference itself");
    }
    match relation {
        WorkRelation::Parent
            if !matches!(
                (source, target),
                (WorkSubject::Activity { .. }, WorkReference::Activity { .. })
            ) =>
        {
            bail!("parent relationship requires activities")
        }
        WorkRelation::MemberOf
            if !matches!(
                (source, target),
                (WorkSubject::Activity { .. }, WorkReference::Session { .. })
            ) =>
        {
            bail!("membership requires an activity and work session")
        }
        _ => Ok(()),
    }
}
