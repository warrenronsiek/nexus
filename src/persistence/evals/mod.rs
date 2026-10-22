// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
// @entrypoint Store::record_eval_bundle
mod blobs;
mod dimensions;
mod invariants;
mod lifecycle;
mod queries;
mod rows;

use self::rows::*;
use super::backend::DbConnection;
use super::schema::{eval_links, eval_records};
use super::Store;
use crate::evals::validation::{identity, MAX_BUNDLE_RECORDS};
use crate::evals::*;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use diesel::prelude::*;
use std::collections::{BTreeMap, HashSet};

impl Store {
    pub fn record_eval_bundle(
        &mut self,
        project: &str,
        records: &[EvalRecordInput],
        actor: &EvalProvenance,
    ) -> Result<EvalReceipt> {
        let prepared = prepare_records(project, records, actor)?;
        self.transaction(|connection| publish(connection, project, records, &prepared))
    }
}

fn prepare_records(
    project: &str,
    records: &[EvalRecordInput],
    actor: &EvalProvenance,
) -> Result<Vec<RecordRow>> {
    identity("project", project)?;
    validate_actor(actor)?;
    if records.is_empty() || records.len() > MAX_BUNDLE_RECORDS {
        bail!("eval bundle requires 1 to 64 records");
    }
    let mut ids = HashSet::new();
    let mut prepared = Vec::new();
    for input in records {
        if !ids.insert(&input.id) {
            bail!("eval bundle contains duplicate record IDs");
        }
        prepared.push(prepare_record(project, input, actor)?);
    }
    Ok(prepared)
}

fn prepare_record(
    project: &str,
    input: &EvalRecordInput,
    actor: &EvalProvenance,
) -> Result<RecordRow> {
    input.validate()?;
    if let Some(source) = &input.source {
        validate_source(source)?;
    }
    let payload = serde_json::to_string(input)?;
    Ok(RecordRow {
        project_id: project.into(),
        id: input.id.clone(),
        kind: input.body.kind().as_str().into(),
        title: input.title.clone(),
        occurred_at: input.occurred_at.to_rfc3339(),
        recorded_at: Utc::now().to_rfc3339(),
        content_hash: blake3::hash(payload.as_bytes()).to_hex().to_string(),
        search_text: format!("{} {}", input.title, payload).to_lowercase(),
        payload_json: payload,
        provenance_json: serde_json::to_string(actor)?,
    })
}

fn validate_source(source: &EvalSource) -> Result<()> {
    identity("source project", &source.project_id)?;
    validate_actor(&source.provenance)
}

fn validate_actor(actor: &EvalProvenance) -> Result<()> {
    identity("agent", &actor.agent)?;
    identity("session", &actor.session_id)?;
    identity("config hash", &actor.config_hash)?;
    if let Some(model) = &actor.model_id {
        identity("actor model", model)?;
    }
    Ok(())
}

fn find(connection: &mut DbConnection, project: &str, id: &str) -> Result<Option<RecordRow>> {
    Ok(eval_records::table
        .find((project, id))
        .select(RecordRow::as_select())
        .first(connection)
        .optional()?)
}

struct Publication<'a> {
    project: &'a str,
    pending: BTreeMap<&'a str, &'a EvalRecordInput>,
    entries: Vec<(&'a EvalRecordInput, &'a RecordRow)>,
    receipt: EvalReceipt,
}

impl<'a> Publication<'a> {
    fn prepare(
        connection: &mut DbConnection,
        project: &'a str,
        records: &'a [EvalRecordInput],
        rows: &'a [RecordRow],
    ) -> Result<Self> {
        let mut publication = Self {
            project,
            pending: records.iter().map(|r| (r.id.as_str(), r)).collect(),
            entries: vec![],
            receipt: EvalReceipt {
                inserted: 0,
                replayed: 0,
                ids: records.iter().map(|r| r.id.clone()).collect(),
            },
        };
        for (input, row) in records.iter().zip(rows) {
            if is_replay(connection, row)? {
                publication.receipt.replayed += 1;
            } else {
                check_references(connection, project, input, &publication.pending)?;
                check_artifacts(connection, project, &input.body, &publication.pending)?;
                publication.entries.push((input, row));
            }
        }
        publication.receipt.inserted = publication.entries.len();
        Ok(publication)
    }
    fn write_records(&self, connection: &mut DbConnection) -> Result<()> {
        for (_, row) in &self.entries {
            diesel::insert_into(eval_records::table)
                .values(*row)
                .execute(connection)?;
        }
        Ok(())
    }
    fn write_links(&self, connection: &mut DbConnection) -> Result<()> {
        for (input, _) in &self.entries {
            for reference in input.body.references() {
                let row = LinkRow {
                    project_id: self.project.into(),
                    source_id: input.id.clone(),
                    target_id: reference.link.target_id,
                    relation: reference.link.relation,
                };
                diesel::insert_into(eval_links::table)
                    .values(&row)
                    .on_conflict_do_nothing()
                    .execute(connection)?;
            }
        }
        Ok(())
    }
    fn write_dimensions(&self, connection: &mut DbConnection) -> Result<()> {
        for (input, _) in &self.entries {
            dimensions::publish(connection, self.project, input, &self.pending)?;
        }
        Ok(())
    }
}

fn is_replay(connection: &mut DbConnection, row: &RecordRow) -> Result<bool> {
    match find(connection, &row.project_id, &row.id)? {
        Some(existing) if existing.content_hash == row.content_hash => Ok(true),
        Some(_) => bail!("immutable eval record conflict: {}", row.id),
        None => Ok(false),
    }
}

fn publish(
    connection: &mut DbConnection,
    project: &str,
    records: &[EvalRecordInput],
    rows: &[RecordRow],
) -> Result<EvalReceipt> {
    let publication = Publication::prepare(connection, project, records, rows)?;
    publication.write_records(connection)?;
    publication.write_links(connection)?;
    publication.write_dimensions(connection)?;
    Ok(publication.receipt)
}

fn check_references(
    connection: &mut DbConnection,
    project: &str,
    input: &EvalRecordInput,
    pending: &BTreeMap<&str, &EvalRecordInput>,
) -> Result<()> {
    for reference in input.body.references() {
        if reference.link.target_id == input.id {
            bail!("eval record cannot reference itself");
        }
        let target = resolve_reference(connection, project, &reference.link.target_id, pending)?;
        if reference
            .expected
            .is_some_and(|kind| kind != target.body.kind())
        {
            bail!(
                "eval reference has wrong record kind: {}",
                reference.link.target_id
            );
        }
        if reference.link.relation == "parent_revision"
            && input.body.revision().map(|v| &v.identity)
                != target.body.revision().map(|v| &v.identity)
        {
            bail!("parent revision belongs to another stable identity");
        }
    }
    invariants::validate_links(connection, project, input, pending)
}

pub(super) fn resolve_reference(
    connection: &mut DbConnection,
    project: &str,
    id: &str,
    pending: &BTreeMap<&str, &EvalRecordInput>,
) -> Result<EvalRecordInput> {
    if let Some(input) = pending.get(id) {
        return Ok((*input).clone());
    }
    Ok(find(connection, project, id)?
        .with_context(|| format!("missing project-local eval reference: {id}"))?
        .record()?
        .input)
}

fn check_artifacts(
    connection: &mut DbConnection,
    project: &str,
    body: &EvalBody,
    pending: &BTreeMap<&str, &EvalRecordInput>,
) -> Result<()> {
    match body {
        EvalBody::Artifact(artifact) => {
            for backing in &artifact.backings {
                check_backing_bytes(connection, project, artifact, backing)?;
            }
            Ok(())
        }
        EvalBody::Verification(verification) => {
            check_verification(connection, project, verification, pending)
        }
        _ => Ok(()),
    }
}

fn check_verification(
    connection: &mut DbConnection,
    project: &str,
    verification: &Verification,
    pending: &BTreeMap<&str, &EvalRecordInput>,
) -> Result<()> {
    let input = resolve_reference(connection, project, &verification.artifact_id, pending)?;
    let EvalBody::Artifact(artifact) = input.body else {
        bail!("verification must reference an artifact");
    };
    match &verification.backing {
        ArtifactBacking::StoredBlob { digest } => {
            check_known_digest(&artifact, digest)?;
            if verification.state == CaptureState::Exact {
                check_backing_bytes(connection, project, &artifact, &verification.backing)?;
            }
        }
        ArtifactBacking::GitLfs { oid, size, .. } => {
            check_known_digest(&artifact, oid)?;
            if *size != artifact.byte_size {
                bail!("verification backing length mismatch");
            }
        }
        ArtifactBacking::GitFile { .. } => {}
    }
    Ok(())
}

fn check_known_digest(artifact: &Artifact, digest: &str) -> Result<()> {
    if artifact
        .digest
        .as_ref()
        .is_some_and(|known| known != digest)
    {
        bail!("verification cannot change known artifact content identity");
    }
    Ok(())
}

fn check_backing_bytes(
    connection: &mut DbConnection,
    project: &str,
    artifact: &Artifact,
    backing: &ArtifactBacking,
) -> Result<()> {
    match backing {
        ArtifactBacking::StoredBlob { digest } => {
            let blob = blobs::find_blob(connection, project, digest)?
                .context("stored artifact blob does not exist")?;
            if !blob.verified
                || blob.byte_size as u64 != artifact.byte_size
                || artifact.digest.as_ref().is_some_and(|d| d != digest)
            {
                bail!("stored artifact bytes must be verified and match declared identity/length");
            }
        }
        ArtifactBacking::GitLfs { oid, size, .. } => {
            if *size != artifact.byte_size || artifact.digest.as_ref().is_some_and(|d| d != oid) {
                bail!("LFS payload identity/size mismatch");
            }
        }
        ArtifactBacking::GitFile { .. } => {}
    }
    Ok(())
}

/// Validate portable evidence before its import transaction can commit.
pub(crate) fn validate_snapshot(connection: &mut DbConnection) -> Result<()> {
    let mut after = (String::new(), String::new());
    loop {
        let rows = eval_records::table
            .filter(
                eval_records::project_id
                    .gt(&after.0)
                    .or(eval_records::project_id
                        .eq(&after.0)
                        .and(eval_records::id.gt(&after.1))),
            )
            .order((eval_records::project_id.asc(), eval_records::id.asc()))
            .limit(16)
            .select(RecordRow::as_select())
            .load::<RecordRow>(connection)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            validate_record_row(connection, &row)?;
            after = (row.project_id, row.id);
        }
    }
    blobs::validate_snapshot(connection)
}

fn validate_record_row(connection: &mut DbConnection, row: &RecordRow) -> Result<()> {
    let record = checked_record(row)?;
    check_references(connection, &row.project_id, &record.input, &BTreeMap::new())?;
    check_artifacts(
        connection,
        &row.project_id,
        &record.input.body,
        &BTreeMap::new(),
    )
}

fn checked_record(row: &RecordRow) -> Result<EvalRecord> {
    let record = row.record()?;
    record.input.validate()?;
    validate_actor(&record.provenance)?;
    validate_row_identity(row, &record)?;
    Ok(record)
}

fn validate_row_identity(row: &RecordRow, record: &EvalRecord) -> Result<()> {
    if row.id != record.input.id
        || row.kind != record.input.body.kind().as_str()
        || row.content_hash != blake3::hash(row.payload_json.as_bytes()).to_hex().as_str()
        || row.title != record.input.title
        || row.occurred_at != record.input.occurred_at.to_rfc3339()
        || row.recorded_at != record.recorded_at.to_rfc3339()
    {
        bail!("portable eval record identity mismatch");
    }
    Ok(())
}
