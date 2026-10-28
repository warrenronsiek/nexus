// @feature code-concerns
// @feature persistence
// @spec docs/features/code-concerns.md
// @spec docs/features/persistence.md
use crate::code_concerns::*;
use crate::persistence::schema::{
    code_concerns, concern_areas, concern_attributions, concern_encounters, concern_observations,
};
use anyhow::Result;
use diesel::prelude::*;

#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name=concern_observations)]
pub(crate) struct ObservationRow {
    pub project_id: String,
    pub id: String,
    pub concern_id: String,
    pub canonical_id: String,
    pub kind: String,
    pub occurred_at: Option<String>,
    pub observed_at: String,
    pub recorded_at: String,
    pub effective_time: String,
    pub payload_json: String,
    pub content_hash: String,
    pub provenance_json: String,
    pub source_json: Option<String>,
    pub reporter_token_hash: Option<String>,
}
impl ObservationRow {
    pub fn observation(&self) -> Result<ConcernObservation> {
        Ok(ConcernObservation {
            input: serde_json::from_str(&self.payload_json)?,
            project_id: self.project_id.clone(),
            recorded_at: self.recorded_at.parse()?,
            provenance: serde_json::from_str(&self.provenance_json)?,
            source: self
                .source_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?,
            reporter_token_hash: self.reporter_token_hash.clone(),
            canonical_id: Some(self.canonical_id.clone()),
        })
    }
    pub fn receipt(&self) -> Result<ConcernReceipt> {
        Ok(ConcernReceipt {
            id: self.id.clone(),
            concern_id: self.concern_id.clone(),
            canonical_id: self.canonical_id.clone(),
            recorded_at: self.recorded_at.parse()?,
            reporter_token: None,
        })
    }
}
#[derive(Clone, Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name=code_concerns)]
pub(super) struct ConcernRow {
    pub project_id: String,
    pub id: String,
    pub canonical_id: String,
    pub fingerprint: String,
    pub title: String,
    pub status: String,
    pub last_observed_at: String,
    pub work_session_count: i64,
    pub returns_count: i64,
    pub summary_json: String,
    pub reporter_token_hash: Option<String>,
}
impl ConcernRow {
    pub fn summary(&self) -> Result<ConcernSummary> {
        Ok(serde_json::from_str(&self.summary_json)?)
    }
}
#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name=concern_encounters)]
pub(super) struct EncounterRow {
    pub project_id: String,
    pub event_id: String,
    pub concern_id: String,
    pub reported_concern_id: String,
    pub source_key: Option<String>,
    pub work_session_id: Option<String>,
    pub host_session_key: Option<String>,
    pub source_time: Option<String>,
    pub effective_time: String,
    pub observed_at: String,
    pub effective_day: String,
    pub repository: Option<String>,
    pub returned_after_resolution: i32,
}
#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name=concern_areas)]
pub(super) struct AreaRow {
    pub project_id: String,
    pub id: String,
    pub repository: String,
    pub kind: String,
    pub area_key: String,
    pub area_json: String,
}
#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name=concern_attributions)]
pub(super) struct AttributionRow {
    pub project_id: String,
    pub event_id: String,
    pub area_id: String,
    pub relation: String,
}
