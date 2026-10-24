// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use crate::persistence::schema::{work_links, work_observations, work_subjects};
use crate::work_history::*;
use anyhow::Result;
use diesel::prelude::*;

#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name=work_observations)]
pub(super) struct ObservationRow {
    pub project_id: String,
    pub id: String,
    pub subject_scope: String,
    pub subject_id: String,
    pub kind: String,
    pub occurred_at: String,
    pub recorded_at: String,
    pub payload_json: String,
    pub content_hash: String,
    pub provenance_json: String,
    pub source_json: Option<String>,
}
impl ObservationRow {
    pub fn replay(&self, expected: &Self) -> Result<WorkReceipt> {
        if self.content_hash != expected.content_hash {
            return Err(WorkRejection::Conflict(expected.id.clone()).into());
        }
        self.receipt()
    }
    pub fn receipt(&self) -> Result<WorkReceipt> {
        Ok(WorkReceipt {
            id: self.id.clone(),
            recorded_at: self.recorded_at.parse()?,
        })
    }
    pub fn observation(&self) -> Result<WorkObservation> {
        Ok(WorkObservation {
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

#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name=work_subjects)]
pub(super) struct SubjectRow {
    pub project_id: String,
    pub scope: String,
    pub id: String,
    pub title: String,
    pub kind: String,
    pub repository: Option<String>,
    pub host_session_id: Option<String>,
    pub environment: Option<String>,
    pub outcome: Option<String>,
    pub occurred_at: String,
    pub recorded_at: String,
}

#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name=work_links)]
pub(super) struct LinkRow {
    pub project_id: String,
    pub event_id: String,
    pub source_scope: String,
    pub source_id: String,
    pub relation: String,
    pub target_scope: String,
    pub target_id: String,
    pub target_json: String,
}
