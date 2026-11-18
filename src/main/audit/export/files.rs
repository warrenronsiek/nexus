// @feature audit-registry
// @spec docs/features/audit-registry.md
// @boundary audit-package-files: check path and digest fields from an offline manifest.
use super::{BlobContents, Manifest};
use anyhow::{ensure, Context, Result};
use nexus::{
    audit::{Finding, RunSummary},
    evals::digest_is_valid,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::Path,
};

const JOURNAL: &str = "journal.jsonl";
const RUNS: &str = "runs.csv";
const FINDINGS: &str = "findings.csv";
const FOLLOW_UP: &str = "followups.csv";
const CHARTS: &str = "charts.csv";
pub(super) const PRIMARY_FILES: [&str; 5] = [JOURNAL, RUNS, FINDINGS, FOLLOW_UP, CHARTS];

pub(super) fn publish(staged: &Path, destination: &Path, manifest: &Manifest) -> Result<()> {
    // An existing destination is never replaced. A completed export is recognizable by its manifest.
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(destination)
        .with_context(|| {
            format!(
                "create new audit export directory {}",
                destination.display()
            )
        })?;
    if staged.join("evidence").is_dir() {
        std::fs::rename(staged.join("evidence"), destination.join("evidence"))?;
    }
    for file in PRIMARY_FILES {
        std::fs::rename(staged.join(file), destination.join(file))?;
    }
    write_manifest(destination, manifest)
}

fn write_manifest(destination: &Path, manifest: &Manifest) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(destination)?;
    serde_json::to_writer_pretty(file.as_file_mut(), manifest)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(destination.join("manifest.json"))?;
    Ok(())
}

pub(super) struct Tables {
    pub(super) journal: BufWriter<File>,
    runs: BufWriter<File>,
    findings: BufWriter<File>,
    pub(super) followups: BufWriter<File>,
    pub(super) charts: BufWriter<File>,
}

impl Tables {
    pub(super) fn new(directory: &Path) -> Result<Self> {
        Ok(Self {
            journal: new_file(&directory.join(JOURNAL))?,
            runs: open_csv(
                directory,
                RUNS,
                &[
                    "run_id",
                    "control",
                    "version",
                    "occurred_at",
                    "subject_json",
                    "execution_status",
                    "duration_ms",
                    "result_code",
                    "result_label",
                    "finding_count",
                    "open_findings",
                ],
            )?,
            findings: open_csv(
                directory,
                FINDINGS,
                &[
                    "run_id",
                    "finding_id",
                    "category",
                    "description",
                    "recommendation",
                ],
            )?,
            followups: open_csv(
                directory,
                FOLLOW_UP,
                &[
                    "run_id",
                    "record_id",
                    "kind",
                    "finding_id",
                    "related_id",
                    "outcome",
                    "reason",
                    "verifier_claim",
                    "reporting_actor",
                    "occurred_at",
                    "recorded_at",
                ],
            )?,
            charts: open_csv(
                directory,
                CHARTS,
                &[
                    "chart", "control", "version", "bucket", "label", "order", "count",
                ],
            )?,
        })
    }

    pub(super) fn finding_row(&mut self, run_id: &str, finding: &Finding) -> Result<()> {
        csv(
            &mut self.findings,
            [
                run_id,
                &finding.id,
                finding.category.as_deref().unwrap_or(""),
                &finding.description,
                finding.recommendation.as_deref().unwrap_or(""),
            ],
        )
    }

    pub(super) fn run_row(&mut self, run: &RunSummary) -> Result<()> {
        csv(
            &mut self.runs,
            [
                &run.id,
                &run.control.key,
                &run.control.version.to_string(),
                &run.occurred_at.to_rfc3339(),
                &serde_json::to_string(&run.subject)?,
                &label(&run.status)?,
                &run.duration_ms
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                run.result_code.as_deref().unwrap_or(""),
                run.result_label.as_deref().unwrap_or(""),
                &run.findings.to_string(),
                &run.open_findings.to_string(),
            ],
        )
    }

    pub(super) fn flush(&mut self) -> Result<()> {
        for file in [
            &mut self.journal,
            &mut self.runs,
            &mut self.findings,
            &mut self.followups,
            &mut self.charts,
        ] {
            file.flush()?;
        }
        Ok(())
    }
}

fn open_csv(directory: &Path, name: &str, header: &[&str]) -> Result<BufWriter<File>> {
    let mut writer = new_file(&directory.join(name))?;
    csv(&mut writer, header.iter().copied())?;
    Ok(writer)
}

pub(super) fn new_file(path: &Path) -> Result<BufWriter<File>> {
    Ok(BufWriter::new(
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?,
    ))
}

pub(super) fn label<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::from_value(serde_json::to_value(value)?)?)
}

pub(super) fn csv<'a>(
    writer: &mut impl Write,
    cells: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    for (index, cell) in cells.into_iter().enumerate() {
        if index > 0 {
            writer.write_all(b",")?;
        }
        let guard = if cell.trim_start().starts_with(['=', '+', '-', '@'])
            || cell.starts_with(['\t', '\r', '\n'])
        {
            "'"
        } else {
            ""
        };
        write!(writer, "\"{guard}{}\"", cell.replace('"', "\"\""))?;
    }
    writer.write_all(b"\n")?;
    Ok(())
}

pub(super) fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        BufReader::new(File::open(path).with_context(|| format!("read {}", path.display()))?);
    let mut hash = Sha256::new();
    let mut buffer = [0; 32 * 1024];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            return Ok(format!("{:x}", hash.finalize()));
        }
        hash.update(&buffer[..size]);
    }
}

pub(crate) fn verify(directory: &Path) -> Result<serde_json::Value> {
    let manifest: Manifest =
        serde_json::from_reader(BufReader::new(File::open(directory.join("manifest.json"))?))?;
    ensure!(
        manifest.format_version == 1,
        "unsupported audit export manifest version"
    );
    for primary in PRIMARY_FILES {
        ensure!(
            manifest.files_sha256.contains_key(primary),
            "audit export manifest lacks {primary}"
        );
    }
    for (name, digest) in &manifest.files_sha256 {
        ensure!(digest_is_valid(digest), "invalid audit export digest");
        ensure!(
            PRIMARY_FILES.contains(&name.as_str())
                || (name == &format!("evidence/{digest}")
                    && matches!(
                        manifest.blob_contents.get(digest),
                        Some(BlobContents::Included)
                    )),
            "unexpected path in audit export manifest"
        );
        ensure!(
            sha256_file(&directory.join(name))? == *digest,
            "audit export digest mismatch for {name}"
        );
    }
    for (digest, state) in &manifest.blob_contents {
        ensure!(
            !matches!(state, BlobContents::Included)
                || manifest
                    .files_sha256
                    .contains_key(&format!("evidence/{digest}")),
            "manifest lacks included audit evidence {digest}"
        );
    }
    Ok(
        serde_json::json!({"verified":true,"files":manifest.files_sha256.len(),"project_id":manifest.project_id,"cutoff":manifest.cutoff}),
    )
}
