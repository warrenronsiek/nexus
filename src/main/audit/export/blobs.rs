// @feature audit-registry
// @spec docs/features/audit-registry.md
// @boundary verified-evidence: decode the existing typed blob-read responses.
use super::files::{new_file, sha256_file};
use crate::history_client::HistoryClient;
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use nexus::{coordination::api::EvalAction, evals::EvalBlob};
use serde::Deserialize;
use std::{io::Write, path::Path};

#[derive(Deserialize)]
struct BlobInfo {
    blob: Option<EvalBlob>,
}

#[derive(Deserialize)]
struct BlobChunk {
    data: Option<String>,
}

pub(super) struct EvidenceBlob<'a> {
    pub(super) project: &'a str,
    pub(super) digest: &'a str,
}

impl EvidenceBlob<'_> {
    pub(super) async fn download(
        &self,
        api: &HistoryClient<EvalAction>,
        directory: &Path,
    ) -> Result<Option<String>> {
        let response = api
            .send(EvalAction::BlobInfo {
                digest: self.digest.into(),
                project_id: Some(self.project.into()),
            })
            .await?;
        let BlobInfo { blob } = serde_json::from_value(response)?;
        let Some(blob) = blob.filter(|blob| blob.verified) else {
            return Ok(None);
        };
        let name = format!("evidence/{}", self.digest);
        let path = directory.join(&name);
        self.write_chunks(api, blob.chunk_count, &path).await?;
        ensure!(
            sha256_file(&path)? == self.digest,
            "audit blob content digest mismatch"
        );
        Ok(Some(name))
    }

    async fn write_chunks(
        &self,
        api: &HistoryClient<EvalAction>,
        count: u32,
        path: &Path,
    ) -> Result<()> {
        std::fs::create_dir_all(path.parent().expect("evidence is inside the export"))?;
        let mut file = new_file(path)?;
        for index in 0..count {
            file.write_all(&self.chunk(api, index).await?)?;
        }
        Ok(file.flush()?)
    }

    async fn chunk(&self, api: &HistoryClient<EvalAction>, index: u32) -> Result<Vec<u8>> {
        let response = api
            .send(EvalAction::BlobGet {
                digest: self.digest.into(),
                index,
                project_id: Some(self.project.into()),
            })
            .await?;
        let BlobChunk { data } = serde_json::from_value(response)?;
        let data = data.with_context(|| {
            format!(
                "verified audit blob {} is missing chunk {index}",
                self.digest
            )
        })?;
        Ok(STANDARD.decode(data)?)
    }
}
