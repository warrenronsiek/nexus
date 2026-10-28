// @feature evals
// @feature runtime
// @spec docs/features/evals.md
// @spec docs/features/runtime.md
// @boundary versioned-eval-reference-bundle
use super::{EvalAction, EvalClient, EvalKind, EvalRecordInput};
use anyhow::{bail, Context, Result};
use nexus::evals::{ArtifactBacking, CaptureState, EvalBody, EvalPage, EvalRecord, EvalSource};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAX_BUNDLE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_LINE_BYTES: u64 = 1024 * 1024;
const MAX_EXPORT_RECORDS: usize = 100_000;

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Frame {
    Header {
        format_version: u32,
        git_dependencies: String,
    },
    Blob {
        digest: String,
        byte_size: u64,
    },
    Chunk {
        digest: String,
        index: u32,
        data: String,
    },
    Seal {
        digest: String,
    },
    Record {
        record: Box<EvalRecordInput>,
    },
    Footer {
        records: usize,
        blobs: usize,
        sha256: String,
    },
}

struct Writer {
    file: tempfile::NamedTempFile,
    hash: Sha256,
    bytes: u64,
    records: usize,
    blobs: usize,
}

impl Writer {
    fn frame(&mut self, frame: &Frame) -> Result<()> {
        let mut bytes = serde_json::to_vec(frame)?;
        bytes.push(b'\n');
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_LINE_BYTES,
            "eval bundle record exceeds 1 MiB"
        );
        self.bytes += bytes.len() as u64;
        anyhow::ensure!(self.bytes <= MAX_BUNDLE_BYTES, "eval bundle exceeds 4 GiB");
        self.hash.update(&bytes);
        self.file.write_all(&bytes)?;
        Ok(())
    }

    fn finish(mut self, output: &Path) -> Result<()> {
        let footer = Frame::Footer {
            records: self.records,
            blobs: self.blobs,
            sha256: format!("{:x}", self.hash.finalize()),
        };
        serde_json::to_writer(&mut self.file, &footer)?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        self.file
            .persist(output)
            .context("publish eval reference bundle")?;
        Ok(())
    }
}

enum Visit {
    Record(String),
    Emit(Box<EvalRecord>),
}

struct ReferenceExport<'a, 'b> {
    client: &'a EvalClient<'b>,
    writer: Writer,
    visited: HashSet<String>,
    active: HashSet<String>,
    blobs: HashSet<String>,
}

impl ReferenceExport<'_, '_> {
    async fn write_history(&mut self) -> Result<()> {
        let mut after = None;
        loop {
            let page = list_page(self.client, after).await?;
            for header in page.records {
                self.walk(header.id).await?;
            }
            after = page.next_cursor;
            if after.is_none() {
                break;
            }
        }
        Ok(())
    }

    async fn walk(&mut self, id: String) -> Result<()> {
        let mut stack = vec![Visit::Record(id)];
        while let Some(item) = stack.pop() {
            match item {
                Visit::Record(id) => self.discover(id, &mut stack).await?,
                Visit::Emit(record) => self.emit(*record).await?,
            }
        }
        Ok(())
    }

    async fn discover(&mut self, id: String, stack: &mut Vec<Visit>) -> Result<()> {
        if self.visited.contains(&id) {
            return Ok(());
        }
        anyhow::ensure!(
            self.active.insert(id.clone()),
            "cyclic eval links cannot be dependency-ordered; use database snapshot export"
        );
        anyhow::ensure!(
            self.active.len() + self.visited.len() <= MAX_EXPORT_RECORDS,
            "eval reference export exceeds 100000 records"
        );
        let record = self.client.get(&id).await?;
        let dependencies = record
            .links
            .iter()
            .map(|l| l.target_id.clone())
            .collect::<Vec<_>>();
        stack.push(Visit::Emit(Box::new(record)));
        stack.extend(dependencies.into_iter().rev().map(Visit::Record));
        Ok(())
    }

    async fn emit(&mut self, mut record: EvalRecord) -> Result<()> {
        self.active.remove(&record.input.id);
        if !self.visited.insert(record.input.id.clone()) {
            return Ok(());
        }
        for digest in blob_references(&record.input.body) {
            if self.blobs.insert(digest.to_owned()) {
                export_blob(self.client, digest, &mut self.writer).await?;
            }
        }
        if record.input.source.is_none() {
            record.input.source = Some(EvalSource {
                project_id: record.project_id,
                recorded_at: record.recorded_at,
                provenance: record.provenance,
            });
        }
        self.writer.frame(&Frame::Record {
            record: Box::new(record.input),
        })?;
        self.writer.records += 1;
        Ok(())
    }
}

async fn list_page(client: &EvalClient<'_>, after: Option<String>) -> Result<EvalPage> {
    let response = client
        .send(EvalAction::List {
            project_id: None,
            kind: None::<EvalKind>,
            repository: None,
            model: None,
            search: None,
            after,
            limit: Some(32),
        })
        .await?;
    serde_json::from_value(
        json!({"records":response["records"],"next_cursor":response["next_cursor"]}),
    )
    .map_err(Into::into)
}

pub(super) async fn export(client: &EvalClient<'_>, output: &Path) -> Result<()> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut writer = Writer {
        file: tempfile::NamedTempFile::new_in(parent)?,
        hash: Sha256::new(),
        bytes: 0,
        records: 0,
        blobs: 0,
    };
    writer.frame(&Frame::Header {
        format_version: 1,
        git_dependencies: "external_pinned_references".into(),
    })?;
    let mut exporter = ReferenceExport {
        client,
        writer,
        visited: HashSet::new(),
        active: HashSet::new(),
        blobs: HashSet::new(),
    };
    exporter.write_history().await?;
    exporter.writer.finish(output)
}

fn blob_references(body: &EvalBody) -> Vec<&str> {
    let backings = match body {
        EvalBody::Artifact(artifact) => artifact.backings.iter().collect::<Vec<_>>(),
        EvalBody::Verification(verification) if verification.state == CaptureState::Exact => {
            vec![&verification.backing]
        }
        _ => Vec::new(),
    };
    backings
        .into_iter()
        .filter_map(|backing| match backing {
            ArtifactBacking::StoredBlob { digest } => Some(digest.as_str()),
            _ => None,
        })
        .collect()
}

async fn export_blob(client: &EvalClient<'_>, digest: &str, writer: &mut Writer) -> Result<()> {
    let blob = client.verified_blob(digest).await?;
    writer.frame(&Frame::Blob {
        digest: digest.into(),
        byte_size: blob.byte_size,
    })?;
    for index in 0..blob.chunk_count {
        export_chunk(client, digest, index, writer).await?;
    }
    writer.frame(&Frame::Seal {
        digest: digest.into(),
    })?;
    writer.blobs += 1;
    Ok(())
}

async fn export_chunk(
    client: &EvalClient<'_>,
    digest: &str,
    index: u32,
    writer: &mut Writer,
) -> Result<()> {
    let response = client
        .send(EvalAction::BlobGet {
            digest: digest.into(),
            index,
            project_id: None,
        })
        .await?;
    let data = response["data"]
        .as_str()
        .context("export blob chunk unavailable")?
        .to_owned();
    writer.frame(&Frame::Chunk {
        digest: digest.into(),
        index,
        data,
    })
}

fn next_line(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_LINE_BYTES + 1)
        .read_until(b'\n', &mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_LINE_BYTES,
        "eval bundle line exceeds 1 MiB"
    );
    Ok((!bytes.is_empty()).then_some(bytes))
}

/// Validate the complete file before the first ingestion; interruptions during ingestion
/// remain replayable because record IDs and blob chunks are immutable and idempotent.
fn validate(reader: &mut impl BufRead) -> Result<(usize, usize)> {
    let mut hash = Sha256::new();
    let mut records = 0;
    let mut blobs = 0;
    let first = next_line(reader)?.context("empty eval bundle")?;
    anyhow::ensure!(
        matches!(serde_json::from_slice::<Frame>(&first)?, Frame::Header {format_version:1, ref git_dependencies} if git_dependencies=="external_pinned_references"),
        "unsupported eval reference bundle header"
    );
    hash.update(first);
    while let Some(bytes) = next_line(reader)? {
        match serde_json::from_slice::<Frame>(&bytes)? {
            Frame::Header { .. } => bail!("duplicate eval bundle header"),
            Frame::Footer {
                records: expected,
                blobs: expected_blobs,
                sha256,
            } => {
                anyhow::ensure!(
                    records == expected && blobs == expected_blobs,
                    "eval bundle counts mismatch"
                );
                anyhow::ensure!(
                    format!("{:x}", hash.finalize()) == sha256,
                    "eval bundle checksum mismatch"
                );
                anyhow::ensure!(
                    next_line(reader)?.is_none(),
                    "data after eval bundle footer"
                );
                return Ok((records, blobs));
            }
            Frame::Record { .. } => records += 1,
            Frame::Blob { .. } => blobs += 1,
            _ => {}
        }
        hash.update(bytes);
    }
    bail!("incomplete eval bundle: footer missing")
}

struct ValidatedBundle {
    reader: BufReader<File>,
    records: usize,
    blobs: usize,
}

impl ValidatedBundle {
    fn open(source: &Path) -> Result<Self> {
        let mut reader = BufReader::new(crate::history_transfer::snapshot_file(
            source,
            MAX_BUNDLE_BYTES,
        )?);
        let (records, blobs) = validate(&mut reader)?;
        reader.seek(SeekFrom::Start(0))?;
        Ok(Self {
            reader,
            records,
            blobs,
        })
    }
    fn next(&mut self) -> Result<Option<Frame>> {
        next_line(&mut self.reader)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(Into::into))
            .transpose()
    }
}

pub(super) async fn import(client: &EvalClient<'_>, source: &Path) -> Result<Value> {
    let mut bundle = ValidatedBundle::open(source)?;
    let mut inserted = 0;
    while let Some(frame) = bundle.next()? {
        if matches!(frame, Frame::Record { .. }) {
            inserted += 1;
        }
        if let Some(action) = import_action(client, frame).await? {
            client.send(action).await.with_context(||format!("import stopped near record {inserted}; previously accepted records remain durable; replay this file to resume"))?;
        }
    }
    Ok(
        json!({"ok":true,"records":bundle.records,"blobs":bundle.blobs,"git_dependencies":"external_pinned_references"}),
    )
}

async fn import_action(client: &EvalClient<'_>, frame: Frame) -> Result<Option<EvalAction>> {
    let action = match frame {
        Frame::Blob { digest, byte_size } => EvalAction::BlobBegin { digest, byte_size },
        Frame::Chunk {
            digest,
            index,
            data,
        } => EvalAction::BlobPut {
            digest,
            index,
            data,
        },
        Frame::Seal { digest } => EvalAction::BlobSeal { digest },
        Frame::Record { record } => EvalAction::Record {
            records: vec![restore_origin_input(client, *record).await?],
        },
        Frame::Header { .. } | Frame::Footer { .. } => return Ok(None),
    };
    Ok(Some(action))
}

async fn restore_origin_input(
    client: &EvalClient<'_>,
    record: EvalRecordInput,
) -> Result<EvalRecordInput> {
    let Some(source) = &record.source else {
        return Ok(record);
    };
    let response = client
        .send(EvalAction::Get {
            id: record.id.clone(),
            project_id: None,
        })
        .await?;
    if response["record"].is_null() {
        return Ok(record);
    }
    let existing: EvalRecord = serde_json::from_value(response["record"].clone())?;
    if existing.input.source.is_none()
        && source.project_id == existing.project_id
        && source.recorded_at == existing.recorded_at
        && source.provenance == existing.provenance
    {
        let mut original = record.clone();
        original.source = None;
        if original == existing.input {
            return Ok(existing.input);
        }
    }
    Ok(record)
}
