// @feature evals
// @feature runtime
// @spec docs/features/evals.md
// @spec docs/features/runtime.md
// @boundary base64-blob-transport
use super::{EvalAction, EvalClient, CHUNK_BYTES};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use nexus::evals::EvalBlob;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

struct UploadSource {
    file: File,
    digest: String,
    size: u64,
}

impl UploadSource {
    fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path).context("open eval artifact")?;
        let size = file.metadata()?.len();
        let digest = hash_file(&mut file)?;
        file.seek(SeekFrom::Start(0))?;
        Ok(Self { file, digest, size })
    }
}

fn hash_file(file: &mut File) -> Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0; CHUNK_BYTES];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

struct VerifiedOutput {
    file: tempfile::NamedTempFile,
    hash: Sha256,
}

impl VerifiedOutput {
    fn new(output: &Path) -> Result<Self> {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        Ok(Self {
            file: tempfile::NamedTempFile::new_in(parent)?,
            hash: Sha256::new(),
        })
    }
    fn append(&mut self, bytes: &[u8]) -> Result<()> {
        self.file.write_all(bytes)?;
        self.hash.update(bytes);
        Ok(())
    }
    fn finish(mut self, digest: &str, output: &Path) -> Result<()> {
        anyhow::ensure!(
            format!("{:x}", self.hash.finalize()) == digest,
            "stored eval blob digest mismatch"
        );
        self.file.flush()?;
        self.file
            .persist(output)
            .context("publish downloaded artifact")?;
        Ok(())
    }
}

impl EvalClient<'_> {
    pub(super) async fn upload(&self, path: &Path) -> Result<Value> {
        let mut source = UploadSource::open(path)?;
        let status = self
            .send(EvalAction::BlobBegin {
                digest: source.digest.clone(),
                byte_size: source.size,
            })
            .await?;
        if status["blob"]["verified"] == true {
            return Ok(status);
        }
        self.upload_chunks(&mut source).await?;
        self.send(EvalAction::BlobSeal {
            digest: source.digest,
        })
        .await
    }

    async fn upload_chunks(&self, source: &mut UploadSource) -> Result<()> {
        let mut buffer = vec![0; CHUNK_BYTES];
        for index in 0..source.size.div_ceil(CHUNK_BYTES as u64) {
            let count =
                ((source.size - index * CHUNK_BYTES as u64).min(CHUNK_BYTES as u64)) as usize;
            source.file.read_exact(&mut buffer[..count])?;
            self.send(EvalAction::BlobPut {
                digest: source.digest.clone(),
                index: u32::try_from(index)?,
                data: STANDARD.encode(&buffer[..count]),
            })
            .await?;
        }
        Ok(())
    }

    pub(super) async fn verified_blob(&self, digest: &str) -> Result<EvalBlob> {
        let response = self
            .send(EvalAction::BlobInfo {
                digest: digest.into(),
                project_id: None,
            })
            .await?;
        let blob: EvalBlob =
            serde_json::from_value(response["blob"].clone()).context("eval blob not found")?;
        anyhow::ensure!(blob.verified, "eval blob is not verified");
        Ok(blob)
    }

    async fn download_chunk(&self, digest: &str, index: u32) -> Result<Vec<u8>> {
        let response = self
            .send(EvalAction::BlobGet {
                digest: digest.into(),
                index,
                project_id: None,
            })
            .await?;
        STANDARD
            .decode(
                response["data"]
                    .as_str()
                    .context("verified eval chunk unavailable")?,
            )
            .context("invalid eval chunk encoding")
    }

    pub(super) async fn download(&self, digest: &str, output: &Path) -> Result<()> {
        let blob = self.verified_blob(digest).await?;
        let mut file = VerifiedOutput::new(output)?;
        for index in 0..blob.chunk_count {
            file.append(&self.download_chunk(digest, index).await?)?;
        }
        file.finish(digest, output)
    }
}
