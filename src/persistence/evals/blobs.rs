// @feature persistence
// @feature evals
// @spec docs/features/persistence.md
// @spec docs/features/evals.md
use super::rows::{BlobRow, ChunkRow};
use crate::evals::{digest_is_valid, EvalBlob, MAX_BLOB_BYTES, MAX_CHUNK_BYTES};
use crate::persistence::backend::DbConnection;
use crate::persistence::schema::{eval_blob_chunks, eval_blobs};
use crate::persistence::Store;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use diesel::prelude::*;
use sha2::{Digest, Sha256};

pub(super) fn find_blob(
    connection: &mut DbConnection,
    project: &str,
    digest: &str,
) -> Result<Option<BlobRow>> {
    Ok(eval_blobs::table
        .find((project, digest))
        .select(BlobRow::as_select())
        .first(connection)
        .optional()?)
}

impl Store {
    pub fn begin_eval_blob(
        &mut self,
        project: &str,
        digest: &str,
        byte_size: u64,
    ) -> Result<EvalBlob> {
        let row = prepare_blob(project, digest, byte_size)?;
        if let Some(existing) = find_blob(&mut self.connection, project, digest)? {
            return replay_blob(&existing, &row);
        }
        diesel::insert_into(eval_blobs::table)
            .values(&row)
            .execute(&mut self.connection)?;
        row.blob()
    }

    pub fn put_eval_blob_chunk(
        &mut self,
        project: &str,
        digest: &str,
        index: u32,
        bytes: &[u8],
    ) -> Result<EvalBlob> {
        let blob = require_blob(&mut self.connection, project, digest)?;
        let chunk = prepare_chunk(&blob, index, bytes)?;
        write_chunk(&mut self.connection, &blob, &chunk)?;
        blob.blob()
    }

    pub fn eval_blob(&mut self, project: &str, digest: &str) -> Result<Option<EvalBlob>> {
        find_blob(&mut self.connection, project, digest)?
            .map(|r| r.blob())
            .transpose()
    }

    pub fn eval_blob_chunk_for_verification(
        &mut self,
        project: &str,
        digest: &str,
        index: u32,
    ) -> Result<Option<Vec<u8>>> {
        let Ok(index) = i32::try_from(index) else {
            return Ok(None);
        };
        Ok(eval_blob_chunks::table
            .find((project, digest, index))
            .select(eval_blob_chunks::bytes)
            .first(&mut self.connection)
            .optional()?)
    }

    pub fn finish_eval_blob_verification(
        &mut self,
        project: &str,
        digest: &str,
        expected_upload_id: &str,
        computed_digest: &str,
    ) -> Result<EvalBlob> {
        self.transaction(|connection| {
            finish_verification(
                connection,
                project,
                digest,
                expected_upload_id,
                computed_digest,
            )
        })
    }

    pub fn abort_eval_blob(&mut self, project: &str, digest: &str) -> Result<bool> {
        self.transaction(|connection| abort_pending(connection, project, digest))
    }

    /// Direct-store convenience. Services use short-lock chunk reads followed by finish.
    pub fn seal_eval_blob(&mut self, project: &str, digest: &str) -> Result<EvalBlob> {
        let blob = self
            .eval_blob(project, digest)?
            .context("unknown blob upload")?;
        let mut hash = Sha256::new();
        for index in 0..blob.chunk_count {
            let chunk = self
                .eval_blob_chunk_for_verification(project, digest, index)?
                .context("blob upload has missing chunks")?;
            hash.update(chunk);
        }
        self.finish_eval_blob_verification(
            project,
            digest,
            &blob.upload_id,
            &format!("{:x}", hash.finalize()),
        )
    }

    pub fn get_eval_blob_chunk(
        &mut self,
        project: &str,
        digest: &str,
        index: u32,
    ) -> Result<Option<Vec<u8>>> {
        if !find_blob(&mut self.connection, project, digest)?.is_some_and(|r| r.verified) {
            return Ok(None);
        }
        self.eval_blob_chunk_for_verification(project, digest, index)
    }
}

pub(super) fn validate_snapshot(connection: &mut DbConnection) -> Result<()> {
    let mut after = (String::new(), String::new());
    loop {
        let rows = eval_blobs::table
            .filter(
                eval_blobs::project_id
                    .gt(&after.0)
                    .or(eval_blobs::project_id
                        .eq(&after.0)
                        .and(eval_blobs::digest.gt(&after.1))),
            )
            .order((eval_blobs::project_id.asc(), eval_blobs::digest.asc()))
            .limit(16)
            .select(BlobRow::as_select())
            .load::<BlobRow>(connection)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            validate_blob_row(connection, &row)?;
            after = (row.project_id, row.digest);
        }
    }
    Ok(())
}

fn prepare_blob(project: &str, digest: &str, byte_size: u64) -> Result<BlobRow> {
    super::identity("project", project)?;
    if !digest_is_valid(digest) || byte_size > MAX_BLOB_BYTES {
        bail!("blob requires SHA-256 and at most {MAX_BLOB_BYTES} bytes");
    }
    Ok(BlobRow {
        project_id: project.into(),
        digest: digest.into(),
        upload_id: uuid::Uuid::new_v4().to_string(),
        byte_size: byte_size.try_into()?,
        chunk_count: byte_size.div_ceil(MAX_CHUNK_BYTES as u64).try_into()?,
        verified: false,
        created_at: Utc::now().to_rfc3339(),
    })
}

fn replay_blob(existing: &BlobRow, incoming: &BlobRow) -> Result<EvalBlob> {
    if existing.byte_size != incoming.byte_size {
        bail!("immutable blob length conflict");
    }
    existing.blob()
}

fn require_blob(connection: &mut DbConnection, project: &str, digest: &str) -> Result<BlobRow> {
    find_blob(connection, project, digest)?.context("unknown blob upload")
}

fn prepare_chunk(blob: &BlobRow, index: u32, bytes: &[u8]) -> Result<ChunkRow> {
    if i64::from(index) >= i64::from(blob.chunk_count) {
        bail!("blob chunk index outside declared content");
    }
    let expected = (blob.byte_size as u64 - u64::from(index) * MAX_CHUNK_BYTES as u64)
        .min(MAX_CHUNK_BYTES as u64);
    if bytes.len() as u64 != expected {
        bail!("blob chunk has incorrect byte length");
    }
    Ok(ChunkRow {
        project_id: blob.project_id.clone(),
        digest: blob.digest.clone(),
        chunk_index: index.try_into()?,
        bytes: bytes.into(),
    })
}

fn write_chunk(connection: &mut DbConnection, blob: &BlobRow, chunk: &ChunkRow) -> Result<()> {
    let existing = eval_blob_chunks::table
        .find((&chunk.project_id, &chunk.digest, chunk.chunk_index))
        .select(eval_blob_chunks::bytes)
        .first::<Vec<u8>>(connection)
        .optional()?;
    match existing {
        Some(bytes) if bytes == chunk.bytes => Ok(()),
        Some(_) => bail!("immutable blob chunk conflict"),
        None if blob.verified => bail!("verified blob cannot receive new chunks"),
        None => diesel::insert_into(eval_blob_chunks::table)
            .values(chunk)
            .execute(connection)
            .map(|_| ())
            .map_err(Into::into),
    }
}

fn validate_blob_row(connection: &mut DbConnection, row: &BlobRow) -> Result<()> {
    let expected = prepare_blob(&row.project_id, &row.digest, u64::try_from(row.byte_size)?)?;
    if expected.chunk_count != row.chunk_count {
        bail!("invalid portable blob chunk count");
    }
    let (count, digest) = hash_chunks(connection, row)?;
    let stored_count = eval_blob_chunks::table
        .filter(eval_blob_chunks::project_id.eq(&row.project_id))
        .filter(eval_blob_chunks::digest.eq(&row.digest))
        .count()
        .get_result::<i64>(connection)?;
    if stored_count != count || row.verified && digest != row.digest {
        bail!("portable blob identity mismatch");
    }
    Ok(())
}

fn hash_chunks(connection: &mut DbConnection, row: &BlobRow) -> Result<(i64, String)> {
    let mut hash = Sha256::new();
    let mut count = 0;
    for index in 0..row.chunk_count {
        if let Some(bytes) = checked_chunk(connection, row, index)? {
            count += 1;
            hash.update(bytes);
        }
    }
    Ok((count, format!("{:x}", hash.finalize())))
}

fn checked_chunk(
    connection: &mut DbConnection,
    row: &BlobRow,
    index: i32,
) -> Result<Option<Vec<u8>>> {
    let bytes = eval_blob_chunks::table
        .find((&row.project_id, &row.digest, index))
        .select(eval_blob_chunks::bytes)
        .first::<Vec<u8>>(connection)
        .optional()?;
    match &bytes {
        Some(value) => {
            prepare_chunk(row, index as u32, value)?;
        }
        None if row.verified => bail!("verified portable blob has missing chunks"),
        None => {}
    }
    Ok(bytes)
}

fn finish_verification(
    connection: &mut DbConnection,
    project: &str,
    digest: &str,
    expected_upload_id: &str,
    computed_digest: &str,
) -> Result<EvalBlob> {
    let mut blob = require_blob(connection, project, digest)?;
    verify_upload_generation(&blob, expected_upload_id, computed_digest)?;
    let count = eval_blob_chunks::table
        .filter(eval_blob_chunks::project_id.eq(project))
        .filter(eval_blob_chunks::digest.eq(digest))
        .count()
        .get_result::<i64>(connection)?;
    if count != i64::from(blob.chunk_count) {
        bail!("blob upload is incomplete");
    }
    let changed = diesel::update(
        eval_blobs::table
            .find((project, digest))
            .filter(eval_blobs::upload_id.eq(expected_upload_id)),
    )
    .set(eval_blobs::verified.eq(true))
    .execute(connection)?;
    if changed != 1 {
        bail!("blob upload generation changed during verification");
    }
    blob.verified = true;
    blob.blob()
}

fn verify_upload_generation(
    blob: &BlobRow,
    expected_upload_id: &str,
    computed_digest: &str,
) -> Result<()> {
    if blob.upload_id != expected_upload_id {
        bail!("blob upload generation changed during verification");
    }
    if computed_digest != blob.digest {
        bail!("blob content digest mismatch");
    }
    Ok(())
}

fn abort_pending(connection: &mut DbConnection, project: &str, digest: &str) -> Result<bool> {
    let Some(blob) = find_blob(connection, project, digest)? else {
        return Ok(false);
    };
    if blob.verified {
        bail!("verified eval evidence cannot be aborted");
    }
    diesel::delete(
        eval_blob_chunks::table
            .filter(eval_blob_chunks::project_id.eq(project))
            .filter(eval_blob_chunks::digest.eq(digest)),
    )
    .execute(connection)?;
    let changed = diesel::delete(
        eval_blobs::table
            .find((project, digest))
            .filter(eval_blobs::verified.eq(false))
            .filter(eval_blobs::upload_id.eq(blob.upload_id)),
    )
    .execute(connection)?;
    if changed != 1 {
        bail!("blob upload changed before abort");
    }
    Ok(true)
}
