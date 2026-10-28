// @feature runtime
// @feature work-history
// @feature code-concerns
// @spec docs/features/runtime.md
// @spec docs/features/work-history.md
// @spec docs/features/code-concerns.md
// @boundary checksummed-observation-exchange: generic frames retain each feature's typed payload and validation
use anyhow::{ensure, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_LINE: usize = 128 * 1024;

#[derive(Clone, Copy)]
pub(super) enum HistoryFormat {
    WorkV1,
    CodeConcernsV1,
}
impl HistoryFormat {
    fn namespace(self) -> Option<String> {
        match self {
            Self::WorkV1 => None,
            Self::CodeConcernsV1 => Some("nexus-code-concerns".into()),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Frame<T> {
    Header {
        format_version: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
    },
    Observation {
        observation: Box<T>,
    },
    Footer {
        count: usize,
        sha256: String,
    },
}

pub(super) struct HistoryWriter<T> {
    file: tempfile::NamedTempFile,
    output: PathBuf,
    hash: Sha256,
    count: usize,
    payload: PhantomData<T>,
}
impl<T: Serialize> HistoryWriter<T> {
    pub(super) fn new(output: &Path, format: HistoryFormat) -> Result<Self> {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut writer = Self {
            file: tempfile::NamedTempFile::new_in(parent)?,
            output: output.to_owned(),
            hash: Sha256::new(),
            count: 0,
            payload: PhantomData,
        };
        writer.frame(&Frame::<T>::Header {
            format_version: 1,
            namespace: format.namespace(),
        })?;
        Ok(writer)
    }
    pub(super) fn push(&mut self, observation: T) -> Result<()> {
        self.frame(&Frame::Observation {
            observation: Box::new(observation),
        })?;
        self.count += 1;
        Ok(())
    }
    fn frame(&mut self, frame: &Frame<T>) -> Result<()> {
        let mut bytes = serde_json::to_vec(frame)?;
        bytes.push(b'\n');
        ensure!(
            bytes.len() <= MAX_LINE,
            "history export line exceeds 128 KiB"
        );
        ensure!(
            self.file.as_file().metadata()?.len() + bytes.len() as u64 <= MAX_BYTES,
            "history export exceeds 1 GiB"
        );
        self.hash.update(&bytes);
        self.file.write_all(&bytes)?;
        Ok(())
    }
    pub(super) fn finish(mut self) -> Result<usize> {
        let footer = Frame::<T>::Footer {
            count: self.count,
            sha256: format!("{:x}", self.hash.clone().finalize()),
        };
        self.frame(&footer)?;
        self.file.flush()?;
        self.file.as_file().sync_all()?;
        self.file
            .persist(&self.output)
            .context("publish observation history export")?;
        Ok(self.count)
    }
}

pub(super) struct VerifiedHistory<T> {
    reader: BufReader<File>,
    pub(super) count: usize,
    payload: PhantomData<T>,
}
impl<T: DeserializeOwned> VerifiedHistory<T> {
    /// Validate a private snapshot completely before yielding the first observation.
    pub(super) fn open(
        path: &Path,
        format: HistoryFormat,
        validate: impl Fn(&T) -> Result<()>,
    ) -> Result<Self> {
        let mut reader = BufReader::new(snapshot_file(path, MAX_BYTES)?);
        let count = validate_stream(&mut reader, format, validate)?;
        reader.seek(SeekFrom::Start(0))?;
        Ok(Self {
            reader,
            count,
            payload: PhantomData,
        })
    }
    pub(super) fn next(&mut self) -> Result<Option<T>> {
        while let Some((frame, _)) = read_frame(&mut self.reader)? {
            if let Frame::Observation { observation } = frame {
                return Ok(Some(*observation));
            }
        }
        Ok(None)
    }
}

pub(super) fn snapshot_file(source: &Path, limit: u64) -> Result<File> {
    let mut snapshot = tempfile::tempfile()?;
    let source = File::open(source)?;
    let copied = std::io::copy(&mut source.take(limit + 1), &mut snapshot)?;
    ensure!(copied <= limit, "history bundle exceeds {limit} bytes");
    snapshot.seek(SeekFrom::Start(0))?;
    Ok(snapshot)
}

fn read_frame<T: DeserializeOwned>(
    reader: &mut BufReader<File>,
) -> Result<Option<(Frame<T>, Vec<u8>)>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    ensure!(
        bytes.len() <= MAX_LINE && bytes.last() == Some(&b'\n'),
        "invalid or oversized history export line"
    );
    Ok(Some((serde_json::from_slice(&bytes)?, bytes)))
}
fn validate_stream<T: DeserializeOwned>(
    reader: &mut BufReader<File>,
    format: HistoryFormat,
    validate: impl Fn(&T) -> Result<()>,
) -> Result<usize> {
    let Some((
        Frame::<T>::Header {
            format_version: 1,
            namespace,
        },
        header,
    )) = read_frame(reader)?
    else {
        anyhow::bail!("unsupported observation history export")
    };
    ensure!(
        namespace == format.namespace(),
        "wrong history feature namespace"
    );
    let mut hash = Sha256::new();
    hash.update(header);
    let mut count = 0;
    loop {
        match read_frame(reader)? {
            Some((Frame::Observation { observation }, bytes)) => {
                validate(&observation)?;
                hash.update(bytes);
                count += 1
            }
            Some((
                Frame::<T>::Footer {
                    count: expected,
                    sha256,
                },
                _,
            )) => {
                ensure!(
                    count == expected && sha256 == format!("{:x}", hash.finalize()),
                    "history export checksum/count mismatch"
                );
                ensure!(
                    read_frame::<T>(reader)?.is_none(),
                    "data after history export footer"
                );
                return Ok(count);
            }
            _ => anyhow::bail!("incomplete or malformed observation history export"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_export_budget_includes_the_footer_and_preserves_previous_output() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("history.jsonl");
        std::fs::write(&output, b"previous complete export").unwrap();
        let mut writer = HistoryWriter::<u8>::new(&output, HistoryFormat::WorkV1).unwrap();
        // A sparse tail exercises the final byte boundary without allocating a GiB.
        writer.file.as_file().set_len(MAX_BYTES - 8).unwrap();
        writer.file.seek(SeekFrom::End(0)).unwrap();
        assert!(
            writer.finish().is_err(),
            "the footer must fit the same importable byte budget"
        );
        assert_eq!(std::fs::read(&output).unwrap(), b"previous complete export");
    }
}
