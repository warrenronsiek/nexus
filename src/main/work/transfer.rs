// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary checksummed-work-history-exchange
use super::WorkClient;
use anyhow::{Context, Result};
use nexus::work_history::{WorkAction, WorkObservation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAX_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_LINE: usize = 128 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Frame {
    Header { format_version: u32 },
    Observation { observation: Box<WorkObservation> },
    Footer { count: usize, sha256: String },
}

struct ExportWriter {
    file: tempfile::NamedTempFile,
    hash: Sha256,
    count: usize,
}

impl ExportWriter {
    fn new(output: &Path) -> Result<Self> {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut writer = Self {
            file: tempfile::NamedTempFile::new_in(parent)?,
            hash: Sha256::new(),
            count: 0,
        };
        write_frame(
            &mut writer.file,
            &mut writer.hash,
            &Frame::Header { format_version: 1 },
        )?;
        Ok(writer)
    }

    fn page(&mut self, page: Value) -> Result<Option<String>> {
        let observations: Vec<WorkObservation> =
            serde_json::from_value(page["observations"].clone())?;
        for observation in observations {
            write_frame(
                &mut self.file,
                &mut self.hash,
                &Frame::Observation {
                    observation: Box::new(observation),
                },
            )?;
            self.count += 1;
        }
        Ok(page["next_cursor"].as_str().map(str::to_owned))
    }

    fn finish(mut self, output: &Path) -> Result<Value> {
        serde_json::to_writer(
            &mut self.file,
            &Frame::Footer {
                count: self.count,
                sha256: format!("{:x}", self.hash.finalize()),
            },
        )?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        self.file
            .persist(output)
            .context("publish work history export")?;
        Ok(json!({"ok":true,"exported":self.count}))
    }
}

pub(super) async fn export(client: &WorkClient, output: &Path) -> Result<Value> {
    let mut writer = ExportWriter::new(output)?;
    let mut after = None;
    loop {
        let page = client
            .send(WorkAction::Export {
                project_id: None,
                after,
                limit: Some(32),
            })
            .await?;
        after = writer.page(page)?;
        if after.is_none() {
            break;
        }
    }
    writer.finish(output)
}

fn write_frame(file: &mut tempfile::NamedTempFile, hash: &mut Sha256, frame: &Frame) -> Result<()> {
    let mut bytes = serde_json::to_vec(frame)?;
    bytes.push(b'\n');
    anyhow::ensure!(bytes.len() <= MAX_LINE, "work export line exceeds 128 KiB");
    anyhow::ensure!(
        file.as_file().metadata()?.len() + bytes.len() as u64 <= MAX_BYTES,
        "work export exceeds 1 GiB"
    );
    hash.update(&bytes);
    file.write_all(&bytes)?;
    Ok(())
}

fn read_frame(reader: &mut BufReader<File>) -> Result<Option<(Frame, Vec<u8>)>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    anyhow::ensure!(
        bytes.len() <= MAX_LINE && bytes.last() == Some(&b'\n'),
        "invalid or oversized work export line"
    );
    Ok(Some((serde_json::from_slice(&bytes)?, bytes)))
}

fn validate(reader: &mut BufReader<File>) -> Result<usize> {
    let Some((Frame::Header { format_version: 1 }, header)) = read_frame(reader)? else {
        anyhow::bail!("unsupported work history export");
    };
    let mut hash = Sha256::new();
    hash.update(header);
    let mut count = 0;
    loop {
        match read_frame(reader)? {
            Some((Frame::Observation { observation }, bytes)) => {
                observation.validate()?;
                hash.update(bytes);
                count += 1;
            }
            Some((
                Frame::Footer {
                    count: expected,
                    sha256,
                },
                _,
            )) => {
                anyhow::ensure!(
                    count == expected && sha256 == format!("{:x}", hash.finalize()),
                    "work export checksum/count mismatch"
                );
                anyhow::ensure!(
                    read_frame(reader)?.is_none(),
                    "data after work export footer"
                );
                return Ok(count);
            }
            _ => anyhow::bail!("incomplete or malformed work history export"),
        }
    }
}

pub(super) async fn import(client: &WorkClient, path: &Path) -> Result<Value> {
    let file = File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.len() <= MAX_BYTES,
        "work export exceeds 1 GiB"
    );
    let mut reader = BufReader::new(file);
    let count = validate(&mut reader)?;
    reader.seek(SeekFrom::Start(0))?;
    let mut imported = 0;
    while let Some((frame, _)) = read_frame(&mut reader)? {
        if let Frame::Observation { observation } = frame {
            // Individual durable acknowledgments make an interrupted import resumable.
            client
                .send(WorkAction::Import {
                    observations: vec![*observation],
                })
                .await?;
            imported += 1;
        }
    }
    Ok(json!({"ok":true,"imported":imported,"validated":count}))
}
