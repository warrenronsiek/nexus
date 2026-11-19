// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
use crate::{delivery::*, persistence::schema::*};
use diesel::prelude::*;
#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name=delivery_observations)]
pub(super) struct ObservationRow {
    pub project_id: String,
    pub id: String,
    pub ordinal: i64,
    pub subject_kind: String,
    pub subject_id: String,
    pub occurred_at: String,
    pub recorded_at: String,
    pub payload_json: String,
    pub content_hash: String,
    pub provenance_json: String,
    pub source_json: Option<String>,
}
impl ObservationRow {
    pub fn observation(&self) -> anyhow::Result<DeliveryObservation> {
        Ok(DeliveryObservation {
            input: serde_json::from_str(&self.payload_json)?,
            project_id: self.project_id.clone(),
            recorded_at: self.recorded_at.parse()?,
            provenance: serde_json::from_str(&self.provenance_json)?,
            source: self
                .source_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?,
        })
    }
}
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name=delivery_records)]
pub(super) struct RecordRow {
    pub project_id: String,
    pub kind: String,
    pub id: String,
    pub repository: String,
    pub title: String,
    pub state: String,
    pub updated_at: String,
    pub recipient_agent: Option<String>,
    pub recipient_session: Option<String>,
    pub payload_json: String,
}
impl RecordRow {
    pub fn record(&self) -> anyhow::Result<DeliveryRecord> {
        Ok(serde_json::from_str(&self.payload_json)?)
    }
}
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name=delivery_origins)]
pub(super) struct OriginRow {
    pub project_id: String,
    pub mapping_id: String,
    pub hunk_index: i32,
    pub contribution_id: String,
    pub repository: String,
    pub revision: String,
    pub path: String,
    pub fingerprint: Option<String>,
    pub new_start: i64,
    pub new_lines: i64,
}
#[derive(Insertable)]
#[diesel(table_name=delivery_relations)]
pub(super) struct RelationRow<'a> {
    pub project_id: &'a str,
    pub source_kind: &'a str,
    pub source_id: &'a str,
    pub target_kind: &'a str,
    pub target_id: &'a str,
}

impl RecordRow {
    pub fn encode(project: &str, record: &DeliveryRecord) -> anyhow::Result<Self> {
        let payload_json = serde_json::to_string(record)?;
        if payload_json.len() > 384 * 1024 {
            anyhow::bail!(DeliveryRejection::Invalid(
                "delivery projection exceeds 384 KiB; split contribution evidence".into()
            ))
        }
        let recipient = match &record.body {
            DeliveryBody::Remediation { remediation, .. } => Some(&remediation.recipient),
            _ => None,
        };
        Ok(Self {
            project_id: project.into(),
            kind: record.subject.kind.as_str().into(),
            id: record.subject.id.clone(),
            repository: record.repository.clone(),
            title: record.title.clone(),
            state: record.state.clone(),
            updated_at: record.updated_at.to_rfc3339(),
            recipient_agent: recipient.map(|a| a.agent.clone()),
            recipient_session: recipient.map(|a| a.session_id.clone()),
            payload_json,
        })
    }
}
impl OriginRow {
    pub fn from_hunk(
        project: &str,
        mapping: &RevisionMapping,
        index: usize,
        hunk: &ChangedHunk,
    ) -> Self {
        Self {
            project_id: project.into(),
            mapping_id: mapping.id.clone(),
            hunk_index: index as i32,
            contribution_id: mapping.contribution_id.clone(),
            repository: mapping.revision.repository.clone(),
            revision: mapping.revision.commit.to_ascii_lowercase(),
            path: hunk.path.clone(),
            fingerprint: hunk.fingerprint.clone(),
            new_start: hunk.new_start.into(),
            new_lines: hunk.new_lines.into(),
        }
    }
}
