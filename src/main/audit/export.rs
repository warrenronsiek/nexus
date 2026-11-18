// @feature audit-registry
// @spec docs/features/audit-registry.md
// @entrypoint export::run
// @boundary audit-export: serialize typed journal records and verify caller-supplied manifest paths.
mod blobs;
mod files;
use super::{send, FilterArgs, HistoryClient};
use anyhow::{bail, ensure, Context, Result};
use blobs::EvidenceBlob;
pub(super) use files::verify;
use files::{csv, label, publish, sha256_file, Tables, PRIMARY_FILES};
use nexus::{
    audit::*,
    coordination::api::{EvalAction, ServiceRequest},
    evals::digest_is_valid,
    runtime::work_capture::CaptureContext,
    LoadedConfig,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, clap::Args)]
pub(crate) struct ExportArgs {
    directory: PathBuf,
    #[command(flatten)]
    scope: FilterArgs,
    /// Include a specific run ID. Repeat to export an explicitly chosen sample.
    #[arg(long = "run")]
    run_ids: Vec<String>,
    /// Include a referenced verified Nexus blob by its SHA-256 digest.
    #[arg(long = "include-blob", conflicts_with = "redact_evidence")]
    include_blobs: Vec<String>,
    /// Replace evidence references with explicit redaction markers.
    #[arg(long)]
    redact_evidence: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format_version: u32,
    project_id: String,
    filter: RunFilter,
    cutoff: i64,
    coverage: String,
    selected_run_ids: Vec<String>,
    counts: Counts,
    references: BTreeMap<String, usize>,
    blob_contents: BTreeMap<String, BlobContents>,
    redactions: BTreeMap<String, usize>,
    files_sha256: BTreeMap<String, String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Counts {
    runs: usize,
    findings: usize,
    followups: usize,
    journal_records: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BlobContents {
    Included,
    OmittedBySelection,
    Unavailable,
}

pub(super) async fn run(
    args: ExportArgs,
    client: &HistoryClient<AuditAction>,
    root: &Path,
    loaded: &LoadedConfig,
    config: Option<&Path>,
) -> Result<serde_json::Value> {
    let blob_api = HistoryClient::new(
        loaded,
        config,
        CaptureContext::from_environment(root).hook,
        ServiceRequest::Evals,
        "audit evidence blob",
    );
    let manifest = args.prepare(client).await?.write(client, &blob_api).await?;
    Ok(serde_json::to_value(manifest)?)
}

struct ExportRequest {
    directory: PathBuf,
    filter: RunFilter,
    project: String,
    cutoff: i64,
    first_page: AuditResponse,
    include: BTreeSet<String>,
    redact: bool,
}

impl ExportArgs {
    async fn prepare(self, client: &HistoryClient<AuditAction>) -> Result<ExportRequest> {
        let mut filter = self.scope.filter();
        filter.run_ids = self
            .run_ids
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        ensure!(
            !filter.run_ids.is_empty() || (filter.from.is_some() && filter.until.is_some()),
            "a period export requires --from and --until, or select run IDs with --run"
        );
        let include: BTreeSet<_> = self.include_blobs.into_iter().collect();
        ensure!(
            include.len() <= 64 && include.iter().all(|digest| digest_is_valid(digest)),
            "invalid selected blob digests"
        );
        let first_page = send(
            client,
            AuditAction::List {
                filter: filter.clone(),
                project_id: self.scope.project,
            },
        )
        .await?;
        let AuditResponse::Page {
            project_id,
            cutoff,
            total,
            ..
        } = &first_page
        else {
            bail!("expected an audit run page");
        };
        ensure!(
            filter.run_ids.is_empty() || usize::try_from(*total).ok() == Some(filter.run_ids.len()),
            "one or more selected audit runs were outside the project or filter"
        );
        Ok(ExportRequest {
            directory: self.directory,
            filter,
            project: project_id.clone(),
            cutoff: *cutoff,
            first_page,
            include,
            redact: self.redact_evidence,
        })
    }
}

impl ExportRequest {
    async fn write(
        self,
        client: &HistoryClient<AuditAction>,
        blobs: &HistoryClient<EvalAction>,
    ) -> Result<Manifest> {
        let parent = self
            .directory
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let staging = tempfile::Builder::new()
            .prefix(".nexus-audit-")
            .tempdir_in(parent)?;
        let destination = self.directory.clone();
        let manifest = self.assemble(staging.path(), client, blobs).await?;
        publish(staging.path(), &destination, &manifest)?;
        Ok(manifest)
    }

    async fn assemble(
        self,
        directory: &Path,
        client: &HistoryClient<AuditAction>,
        blobs: &HistoryClient<EvalAction>,
    ) -> Result<Manifest> {
        let mut package = Package::new(
            directory,
            self.project,
            self.cutoff,
            self.filter,
            self.redact,
        )?;
        package.collect(client, self.first_page).await?;
        package.include_blobs(blobs, self.include).await?;
        package.finish()
    }
}

struct Package<'a> {
    directory: &'a Path,
    output: Tables,
    manifest: Manifest,
    controls: BTreeSet<String>,
    charts: BTreeMap<ChartKey, usize>,
    redact: bool,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ChartKey {
    chart: &'static str,
    control: String,
    version: i32,
    order: Option<i32>,
    bucket: String,
    label: String,
}

struct ChartBucket<'a> {
    chart: &'static str,
    code: &'a str,
    label: &'a str,
    order: Option<i32>,
}

impl<'a> ChartBucket<'a> {
    fn named(chart: &'static str, name: &'a str) -> Self {
        Self {
            chart,
            code: name,
            label: name,
            order: None,
        }
    }

    fn unrecorded(chart: &'static str) -> Self {
        Self {
            chart,
            code: "unrecorded",
            label: "No disposition recorded",
            order: None,
        }
    }
}

impl<'a> Package<'a> {
    fn new(
        directory: &'a Path,
        project_id: String,
        cutoff: i64,
        filter: RunFilter,
        redact: bool,
    ) -> Result<Self> {
        Ok(Self {
            directory,
            output: Tables::new(directory)?,
            controls: BTreeSet::new(),
            charts: BTreeMap::new(),
            redact,
            manifest: Manifest {
                format_version: 1,
                project_id,
                cutoff,
                filter,
                coverage: "unknown_recorded_runs_only".into(),
                selected_run_ids: Vec::new(),
                counts: Counts::default(),
                references: BTreeMap::new(),
                blob_contents: BTreeMap::new(),
                redactions: BTreeMap::new(),
                files_sha256: BTreeMap::new(),
            },
        })
    }

    async fn collect(
        &mut self,
        client: &HistoryClient<AuditAction>,
        mut page: AuditResponse,
    ) -> Result<()> {
        loop {
            let AuditResponse::Page {
                items, next_cursor, ..
            } = page
            else {
                bail!("expected audit run page");
            };
            for run in items {
                if self.controls.insert(run.control.key.clone()) {
                    self.control_history(client, &run.control.key).await?;
                }
                self.run_history(client, &run).await?;
                self.record_charts(&run)?;
                self.manifest.selected_run_ids.push(run.id);
                self.manifest.counts.runs += 1;
            }
            let Some(cursor) = next_cursor else {
                return Ok(());
            };
            let filter = RunFilter {
                after: Some(cursor),
                cutoff: Some(self.manifest.cutoff),
                ..self.manifest.filter.clone()
            };
            page = send(
                client,
                AuditAction::List {
                    project_id: Some(self.manifest.project_id.clone()),
                    filter,
                },
            )
            .await?;
        }
    }

    async fn control_history(
        &mut self,
        client: &HistoryClient<AuditAction>,
        key: &str,
    ) -> Result<()> {
        let mut after = None;
        loop {
            let AuditResponse::Journal {
                records,
                next_cursor,
                ..
            } = send(
                client,
                AuditAction::Journal {
                    project_id: Some(self.manifest.project_id.clone()),
                    control_key: Some(key.into()),
                    cutoff: Some(self.manifest.cutoff),
                    after,
                },
            )
            .await?
            else {
                bail!("expected audit control history");
            };
            for record in records {
                self.record(record)?;
            }
            let Some(cursor) = next_cursor else {
                return Ok(());
            };
            after = Some(cursor);
        }
    }

    async fn run_history(
        &mut self,
        client: &HistoryClient<AuditAction>,
        summary: &RunSummary,
    ) -> Result<()> {
        let mut after = None;
        let mut latest = None;
        loop {
            let AuditResponse::Detail {
                records,
                next_cursor,
                ..
            } = send(
                client,
                AuditAction::Get {
                    project_id: Some(self.manifest.project_id.clone()),
                    run_id: summary.id.clone(),
                    cutoff: Some(self.manifest.cutoff),
                    after,
                },
            )
            .await?
            else {
                bail!("expected audit run history");
            };
            for record in records {
                self.run_record(summary, record, &mut latest)?;
            }
            let Some(cursor) = next_cursor else {
                break;
            };
            after = Some(cursor);
        }
        self.output.run_row(summary)?;
        self.write_findings(
            &summary.id,
            latest.context("audit run history lacks its original run")?,
        )
    }

    fn run_record(
        &mut self,
        summary: &RunSummary,
        record: AuditRecord,
        latest: &mut Option<Vec<Finding>>,
    ) -> Result<()> {
        match &record.input.event {
            AuditEvent::Run { run, .. } => *latest = Some(run.result.findings.clone()),
            AuditEvent::CorrectRun { result, .. } => *latest = Some(result.findings.clone()),
            AuditEvent::Disposition { .. } | AuditEvent::Verification { .. } => {
                self.followup(&summary.id, &record)?
            }
            _ => bail!("unexpected record in audit run history"),
        }
        self.record(record)
    }

    fn write_findings(&mut self, run_id: &str, findings: Vec<Finding>) -> Result<()> {
        for finding in findings {
            self.output.finding_row(run_id, &finding)?;
            self.manifest.counts.findings += 1;
        }
        Ok(())
    }

    fn followup(&mut self, run_id: &str, record: &AuditRecord) -> Result<()> {
        let (kind, finding, related, status, reason, verifier) = match &record.input.event {
            AuditEvent::Disposition {
                finding_id,
                previous,
                disposition,
                reason,
                ..
            } => (
                "disposition",
                finding_id.as_deref(),
                previous.as_deref(),
                label(disposition)?,
                reason,
                None,
            ),
            AuditEvent::Verification {
                disposition_id,
                outcome,
                reason,
                verifier,
                ..
            } => (
                "verification",
                None,
                Some(disposition_id.as_str()),
                label(outcome)?,
                reason,
                Some(verifier.as_str()),
            ),
            _ => unreachable!("called for a follow-up"),
        };
        csv(
            &mut self.output.followups,
            [
                run_id,
                &record.input.id,
                kind,
                finding.unwrap_or(""),
                related.unwrap_or(""),
                &status,
                reason,
                verifier.unwrap_or(""),
                &record.actor.agent,
                &record.input.occurred_at.to_rfc3339(),
                &record.recorded_at.to_rfc3339(),
            ],
        )?;
        self.manifest.counts.followups += 1;
        Ok(())
    }

    fn record(&mut self, mut record: AuditRecord) -> Result<()> {
        let mut redacted = 0;
        for item in record.input.event.evidence_lists().flatten() {
            if self.redact {
                redacted += 1;
                continue;
            }
            let state = match item {
                AuditEvidence::StoredBlob { digest, .. } => {
                    self.manifest
                        .blob_contents
                        .entry(digest.clone())
                        .or_insert(BlobContents::OmittedBySelection);
                    "nexus_verified_blob"
                }
                AuditEvidence::Unavailable { .. } => "unavailable",
                _ => "caller_reference_content_omitted",
            };
            *self.manifest.references.entry(state.into()).or_insert(0) += 1;
        }
        if redacted != 0 {
            self.manifest
                .redactions
                .insert(record.input.id.clone(), redacted);
            record.input.event.redact_evidence();
        }
        serde_json::to_writer(&mut self.output.journal, &record)?;
        self.output.journal.write_all(b"\n")?;
        self.manifest.counts.journal_records += 1;
        Ok(())
    }

    fn record_charts(&mut self, run: &RunSummary) -> Result<()> {
        let day = run.occurred_at.date_naive().to_string();
        self.chart(run, ChartBucket::named("daily_runs", &day), 1);
        self.chart(run, ChartBucket::named("control_runs", &run.control.key), 1);
        self.chart(
            run,
            ChartBucket::named("execution_status", &label(&run.status)?),
            1,
        );
        if let (Some(code), Some(label)) = (&run.result_code, &run.result_label) {
            self.chart(
                run,
                ChartBucket {
                    chart: "categorized_results",
                    code,
                    label,
                    order: run.result_order,
                },
                1,
            );
        }
        self.disposition_charts(run)
    }

    fn disposition_charts(&mut self, run: &RunSummary) -> Result<()> {
        let status = run.disposition.map(|state| label(&state)).transpose()?;
        let bucket = status.as_deref().map_or_else(
            || ChartBucket::unrecorded("run_disposition"),
            |value| ChartBucket::named("run_disposition", value),
        );
        self.chart(run, bucket, 1);
        for (status, count) in &run.finding_dispositions {
            self.chart(
                run,
                ChartBucket::named("finding_disposition", &label(status)?),
                *count,
            );
        }
        let unresolved = run
            .findings
            .saturating_sub(run.finding_dispositions.values().sum());
        if unresolved > 0 {
            self.chart(
                run,
                ChartBucket::unrecorded("finding_disposition"),
                unresolved,
            );
        }
        Ok(())
    }

    fn chart(&mut self, run: &RunSummary, bucket: ChartBucket<'_>, count: usize) {
        *self
            .charts
            .entry(ChartKey {
                chart: bucket.chart,
                control: run.control.key.clone(),
                version: run.control.version,
                order: bucket.order,
                bucket: bucket.code.into(),
                label: bucket.label.into(),
            })
            .or_default() += count;
    }

    async fn include_blobs(
        &mut self,
        api: &HistoryClient<EvalAction>,
        selected: BTreeSet<String>,
    ) -> Result<()> {
        ensure!(
            selected
                .iter()
                .all(|digest| self.manifest.blob_contents.contains_key(digest)),
            "selected blob was not referenced by an exported record"
        );
        for digest in selected {
            let blob = EvidenceBlob {
                project: &self.manifest.project_id,
                digest: &digest,
            };
            let contents = if let Some(path) = blob.download(api, self.directory).await? {
                self.manifest.files_sha256.insert(path, digest.clone());
                BlobContents::Included
            } else {
                BlobContents::Unavailable
            };
            self.manifest.blob_contents.insert(digest, contents);
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Manifest> {
        for (key, count) in self.charts {
            csv(
                &mut self.output.charts,
                [
                    key.chart,
                    &key.control,
                    &key.version.to_string(),
                    &key.bucket,
                    &key.label,
                    &key.order.map(|order| order.to_string()).unwrap_or_default(),
                    &count.to_string(),
                ],
            )?;
        }
        self.output.flush()?;
        for name in PRIMARY_FILES {
            self.manifest
                .files_sha256
                .insert(name.into(), sha256_file(&self.directory.join(name))?);
        }
        Ok(self.manifest)
    }
}
