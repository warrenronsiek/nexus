// @feature audit-registry
// @feature commit-review
// @spec docs/features/audit-registry.md
// @spec docs/features/commit-review.md
use super::{audit_client, send, HistoryClient, PreparedAuditRecord};
use anyhow::{bail, ensure, Context, Result};
use nexus::{
    agents::reviewer,
    audit::{AuditAction, AuditEvent, AuditResponse, ControlVersion, ResultSchema},
    LoadedConfig,
};
use std::path::{Path, PathBuf};

#[derive(Debug, clap::Args)]
pub(crate) struct ReviewArgs {
    /// Record this invocation against an explicitly registered audit control version.
    #[arg(long, requires = "audit_record", value_parser = parse_control)]
    audit_control: Option<ControlVersion>,
    /// Write a private replayable audit record to this new path.
    #[arg(long, requires = "audit_control")]
    audit_record: Option<PathBuf>,
}

fn parse_control(value: &str) -> Result<ControlVersion> {
    let (key, version) = value.rsplit_once('@').context("use <key>@<version>")?;
    Ok(ControlVersion {
        key: key.into(),
        version: version.parse().context("invalid control version")?,
    })
}

impl ReviewArgs {
    pub(crate) async fn run(
        self,
        root: &Path,
        loaded: &LoadedConfig,
        config: Option<&Path>,
    ) -> Result<()> {
        match (self.audit_control, self.audit_record) {
            (Some(control), Some(path)) => audited(control, &path, root, loaded, config).await,
            (None, None) => ordinary(root, loaded, config).await,
            _ => unreachable!("clap requires paired audit review flags"),
        }
    }
}

async fn ordinary(root: &Path, loaded: &LoadedConfig, config: Option<&Path>) -> Result<()> {
    let report = reviewer::review_staged_commit_recorded(loaded, root);
    let _ = nexus::runtime::work_capture::flush_before_exit(loaded, config).await;
    print_report(&report?)
}

fn print_report(report: &reviewer::CommitReviewReport) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(report)?);
    Ok(())
}

async fn audited(
    control: ControlVersion,
    path: &Path,
    root: &Path,
    loaded: &LoadedConfig,
    config: Option<&Path>,
) -> Result<()> {
    let client = audit_client(root, loaded, config);
    let project_id = validate_control(&client, control.clone()).await?;
    let receipt = reserve(path)?;
    let reviewed = reviewer::review_staged_commit_audited(loaded, root, control);
    let saved = PreparedAuditRecord {
        project_id,
        record: reviewed.record,
    };
    save(receipt, path, &saved)?;
    if let Ok(report) = &reviewed.report {
        print_report(report)?;
    }
    let delivery = client.send(saved.into_action()).await.with_context(|| {
        format!(
            "review finished. Retry delivery with nexus audit append {}",
            path.display()
        )
    });
    let _ = nexus::runtime::work_capture::flush_before_exit(loaded, config).await;
    eprintln!("{}", delivery?);
    reviewed.report.map(|_| ())
}

async fn validate_control(
    client: &HistoryClient<AuditAction>,
    control: ControlVersion,
) -> Result<String> {
    let AuditResponse::Control {
        project_id,
        definition: Some(definition),
        retired,
    } = send(
        client,
        AuditAction::Control {
            control,
            project_id: None,
        },
    )
    .await?
    else {
        bail!("register the audit control version before running the review");
    };
    ensure!(!retired, "the specified audit control version is retired");
    let AuditEvent::Define { definition, .. } = definition.input.event else {
        bail!("invalid audit control response");
    };
    ensure!(
        matches!(&definition.results, ResultSchema::Categorical { codes }
        if ["pass", "changes_requested", "inconclusive"].iter().all(|key| codes.iter().any(|code| code.code == *key))),
        "the review adapter requires pass, changes_requested and inconclusive result codes"
    );
    Ok(project_id)
}

fn reserve(path: &Path) -> Result<tempfile::NamedTempFile> {
    ensure!(
        path.symlink_metadata().is_err(),
        "audit replay file already exists"
    );
    let directory = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    tempfile::NamedTempFile::new_in(directory).context("reserve a private review replay file")
}

fn save(
    mut receipt: tempfile::NamedTempFile,
    path: &Path,
    saved: &PreparedAuditRecord,
) -> Result<()> {
    serde_json::to_writer(receipt.as_file_mut(), saved)?;
    receipt.as_file().sync_all()?;
    receipt
        .persist_noclobber(path)
        .with_context(|| format!("save audit record at {}", path.display()))?;
    Ok(())
}
