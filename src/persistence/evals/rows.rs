// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
use crate::evals::*;
use crate::persistence::schema::{
    eval_blob_chunks, eval_blobs, eval_dimensions, eval_links, eval_records,
};
use anyhow::Result;
use diesel::prelude::*;

#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = eval_records)]
pub(super) struct RecordRow {
    pub project_id: String,
    pub id: String,
    pub kind: String,
    pub title: String,
    pub occurred_at: String,
    pub recorded_at: String,
    pub payload_json: String,
    pub content_hash: String,
    pub provenance_json: String,
    pub search_text: String,
}

impl RecordRow {
    pub fn record(&self) -> Result<EvalRecord> {
        let input: EvalRecordInput = serde_json::from_str(&self.payload_json)?;
        let links = input
            .body
            .references()
            .into_iter()
            .map(|r| r.link)
            .collect();
        Ok(EvalRecord {
            input,
            project_id: self.project_id.clone(),
            recorded_at: self.recorded_at.parse()?,
            provenance: serde_json::from_str(&self.provenance_json)?,
            links,
            lifecycle: None,
        })
    }
}

#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = eval_links)]
pub(super) struct LinkRow {
    pub project_id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
}

#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = eval_dimensions)]
pub(super) struct DimensionRow {
    pub project_id: String,
    pub record_id: String,
    pub dimension: String,
    pub value: String,
}

#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = eval_blobs)]
pub(super) struct BlobRow {
    pub project_id: String,
    pub digest: String,
    pub upload_id: String,
    pub byte_size: i64,
    pub chunk_count: i32,
    pub verified: bool,
    pub created_at: String,
}

impl BlobRow {
    pub fn blob(&self) -> Result<EvalBlob> {
        Ok(EvalBlob {
            digest: self.digest.clone(),
            upload_id: self.upload_id.clone(),
            byte_size: self.byte_size.try_into()?,
            chunk_count: self.chunk_count.try_into()?,
            verified: self.verified,
        })
    }
}

#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = eval_blob_chunks)]
pub(super) struct ChunkRow {
    pub project_id: String,
    pub digest: String,
    pub chunk_index: i32,
    pub bytes: Vec<u8>,
}
