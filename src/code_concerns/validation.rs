// @feature code-concerns
// @spec docs/features/code-concerns.md
use super::*;
use anyhow::{bail, Result};
pub(crate) fn text(value: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max || value.contains('\0') {
        bail!("invalid bounded concern text")
    }
    Ok(())
}
impl ConcernActor {
    pub fn validate(&self) -> Result<()> {
        text(&self.agent, 256)?;
        text(&self.session_id, 512)?;
        text(&self.config_hash, 512)?;
        self.model_id.iter().try_for_each(|v| text(v, 512))
    }
}
impl ConcernSource {
    fn validate(&self) -> Result<()> {
        text(&self.project_id, 512)?;
        self.provenance.validate()
    }
}
impl ConcernObservation {
    pub fn validate(&self) -> Result<()> {
        self.input.validate()?;
        self.provenance.validate()?;
        self.source.iter().try_for_each(ConcernSource::validate)?;
        self.validate_receipt()
    }
    fn validate_receipt(&self) -> Result<()> {
        text(&self.project_id, 512)?;
        self.canonical_id.iter().try_for_each(|v| text(v, 512))?;
        if let Some(hash) = &self.reporter_token_hash {
            if hash.len() != 64 || !hash.bytes().all(|v| v.is_ascii_hexdigit()) {
                bail!("invalid reporter capability hash")
            }
        }
        Ok(())
    }
}
impl ConcernQuery {
    pub fn validate(&self) -> Result<()> {
        [&self.repository, &self.area_id, &self.search]
            .into_iter()
            .flatten()
            .try_for_each(|v| text(v, 1024))?;
        self.after.iter().try_for_each(|v| text(v, 4096))?;
        if matches!((self.occurred_after,self.occurred_before),(Some(a),Some(b)) if a>=b) {
            bail!("concern window must be increasing")
        }
        Ok(())
    }
}
impl ConcernAction {
    pub fn validate_read(&self) -> Result<()> {
        let result = match self {
            Self::List { query, .. } | Self::Dashboard { query, .. } => query.validate(),
            Self::Get { concern_id, .. } => text(concern_id, 512),
            Self::History {
                concern_id, after, ..
            } => {
                text(concern_id, 512)?;
                after.iter().try_for_each(|v| text(v, 4096))
            }
            Self::Export { after, .. } => after.iter().try_for_each(|v| text(v, 4096)),
            Self::Respond {
                observation,
                reporter_token,
            } => {
                if !matches!(observation.body, ConcernEvent::DuplicateResponded { .. }) {
                    bail!("respond requires duplicate_responded observation")
                }
                text(reporter_token, 512)
            }
            _ => Ok(()),
        };
        result?;
        Ok(())
    }
}
impl ConcernObservationInput {
    pub fn validate(&self) -> Result<()> {
        text(&self.id, 512)?;
        text(&self.concern_id, 512)?;
        if serde_json::to_vec(self)?.len() > 16 * 1024 {
            bail!("concern observation exceeds16KiB")
        }
        [
            &self.context.host_session_id,
            &self.context.work_session_id,
            &self.context.activity_id,
            &self.context.repository,
        ]
        .into_iter()
        .flatten()
        .try_for_each(|v| text(v, 512))?;
        if let Some(encounter) = self.body.encounter() {
            encounter.validate()?
        }
        self.body.validate()
    }
}
impl ConcernArea {
    pub fn validate(&self) -> Result<()> {
        text(&self.repository, 512)?;
        text(&self.key, 1024)?;
        text(&self.display_name, 512)?;
        self.id.iter().try_for_each(|v| text(v, 512))?;
        if self.kind == ConcernAreaKind::File {
            relative(&self.key)?
        }
        if self.kind == ConcernAreaKind::Declaration && self.discriminator.is_none() {
            bail!("declaration needs a path/signature discriminator")
        }
        self.discriminator.iter().try_for_each(|v| text(v, 1024))
    }
}
fn relative(path: &str) -> Result<()> {
    text(path, 4096)?;
    if path.starts_with('/')
        || path.contains('\\')
        || path.split('/').any(|part| part == ".." || part.is_empty())
    {
        bail!("location requires repository-relative path")
    }
    Ok(())
}
impl ConcernLocation {
    pub fn validate(&self) -> Result<()> {
        text(&self.repository, 512)?;
        self.path.iter().try_for_each(|v| relative(v))?;
        match (&self.commit, &self.object_format) {
            (Some(commit), Some(format)) => {
                let length = match format.as_str() {
                    "sha1" => 40,
                    "sha256" => 64,
                    _ => bail!("unknown Git object format"),
                };
                if commit.len() != length || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
                    bail!("Git commit requires full object ID")
                }
            }
            (None, None) => (),
            _ => bail!("commit and object format must be supplied together"),
        }
        if self.line_start == Some(0)
            || self.line_end == Some(0)
            || matches!((self.line_start,self.line_end),(Some(a),Some(b))if b<a)
        {
            bail!("invalid location line range")
        }
        [&self.language, &self.qualified_name, &self.declaration_kind]
            .into_iter()
            .flatten()
            .try_for_each(|v| text(v, 1024))
    }
}
impl ConcernEncounter {
    fn validate(&self) -> Result<()> {
        if self.areas.len() > 16 || self.evidence.len() > 16 {
            bail!("too many concern attributions")
        }
        [&self.symptom, &self.consequence, &self.workaround]
            .into_iter()
            .flatten()
            .try_for_each(|v| text(v, 4096))?;
        if let Some(source) = &self.source {
            for value in [
                &source.source,
                &source.repository,
                &source.execution_attempt,
                &source.failure_id,
            ] {
                text(value, 512)?
            }
        }
        for attribution in &self.areas {
            attribution.area.validate()?;
            attribution
                .location
                .iter()
                .try_for_each(ConcernLocation::validate)?;
            if attribution.memberships.len() > 16 {
                bail!("too many area memberships")
            }
            attribution
                .memberships
                .iter()
                .try_for_each(ConcernArea::validate)?;
        }
        self.evidence.iter().try_for_each(ConcernEvidence::validate)
    }
}
impl ConcernEvidence {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Concern { id }
            | Self::Papercut { id }
            | Self::Sos { id }
            | Self::Eval { id }
            | Self::WorkSession { id }
            | Self::WorkActivity { id } => text(id, 512),
            Self::External { uri, description } => {
                text(uri, 4096)?;
                text(description, 2048)
            }
            Self::Git { location } => location.validate(),
            Self::Artifact {
                backing,
                description,
                ..
            } => {
                crate::evals::validation::validate_backing(backing)?;
                text(description, 2048)
            }
        }
    }
}
impl ConcernEvent {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Reported {
                title,
                description,
                hypothesis,
                impact,
                ..
            } => report(title, description, hypothesis, impact),
            Self::Encountered { .. } => Ok(()),
            Self::EvidenceAdded {
                encounter_id,
                note,
                evidence,
            } => evidence_note(encounter_id, note, evidence),
            Self::RemedyRecorded {
                encounter_ids,
                what,
                why,
                outcome,
                evidence,
            } => note(encounter_ids, &[what, why, outcome], evidence),
            Self::ReviewRecorded {
                encounter_ids,
                hypothesis,
                rationale,
                alternatives,
                evidence,
            } => review(encounter_ids, hypothesis, rationale, alternatives, evidence),
            Self::Resolved {
                reason,
                fixing_location,
                evidence,
            } => resolution(reason, fixing_location, evidence),
            Self::Reopened { reason, evidence } => note(&[], &[reason], evidence),
            Self::AreaMapped {
                from,
                to,
                reason,
                replaces,
                ..
            } => mapping(from, to, reason, replaces),
            Self::DuplicateProposed {
                target_concern_id,
                rationale,
            } => explained_reference(target_concern_id, rationale),
            Self::DuplicateResponded {
                proposal_id,
                reason,
                ..
            } => explained_reference(proposal_id, reason),
        }
    }
}
fn report(
    title: &str,
    description: &str,
    hypothesis: &Option<String>,
    impact: &Option<String>,
) -> Result<()> {
    text(title, 512)?;
    text(description, 4096)?;
    [hypothesis, impact]
        .into_iter()
        .flatten()
        .try_for_each(|v| text(v, 4096))
}
fn evidence_note(
    encounter: &Option<String>,
    value: &str,
    evidence: &[ConcernEvidence],
) -> Result<()> {
    encounter.iter().try_for_each(|v| text(v, 512))?;
    text(value, 4096)?;
    evidence.iter().try_for_each(ConcernEvidence::validate)
}
fn resolution(
    reason: &String,
    location: &Option<ConcernLocation>,
    evidence: &[ConcernEvidence],
) -> Result<()> {
    location.iter().try_for_each(ConcernLocation::validate)?;
    note(&[], &[reason], evidence)
}
fn mapping(
    from: &ConcernArea,
    to: &ConcernArea,
    reason: &str,
    replaces: &Option<String>,
) -> Result<()> {
    from.validate()?;
    to.validate()?;
    text(reason, 4096)?;
    replaces.iter().try_for_each(|v| text(v, 512))
}
fn explained_reference(id: &str, reason: &str) -> Result<()> {
    text(id, 512)?;
    text(reason, 4096)
}
fn review(
    ids: &[String],
    hypothesis: &String,
    rationale: &String,
    alternatives: &[String],
    evidence: &[ConcernEvidence],
) -> Result<()> {
    if alternatives.len() > 16 {
        bail!("too many alternatives")
    }
    alternatives
        .iter()
        .try_for_each(|value| text(value, 2048))?;
    note(ids, &[hypothesis, rationale], evidence)
}
fn note(ids: &[String], text_values: &[&String], evidence: &[ConcernEvidence]) -> Result<()> {
    if ids.len() > 32 || evidence.len() > 16 {
        bail!("too many concern references")
    }
    ids.iter().try_for_each(|v| text(v, 512))?;
    text_values.iter().try_for_each(|v| text(v, 4096))?;
    evidence.iter().try_for_each(ConcernEvidence::validate)
}
