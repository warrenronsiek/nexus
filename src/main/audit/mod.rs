// @feature audit-registry
// @spec docs/features/audit-registry.md
// @boundary audit-cli-json: decode bounded caller-owned typed requests.
mod export;
mod review;
use crate::history_client::HistoryClient;
use anyhow::{ensure, Context, Result};
use clap::Subcommand;
use nexus::{
    audit::{AuditAction, AuditInput, AuditResponse, RunFilter, MAX_RECORD_BYTES},
    coordination::api::ServiceRequest,
    runtime::work_capture::CaptureContext,
    LoadedConfig,
};
pub(super) use review::ReviewArgs;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[derive(Debug, Subcommand)]
pub(super) enum AuditCliCommand {
    /// Print the typed CLI and MCP contract.
    Schema,
    /// Append one immutable record from a file or stdin (-).
    Append { file: PathBuf },
    /// Run a typed read from a file or stdin (-).
    Read { file: PathBuf },
    /// List recent recorded executions.
    List(FilterArgs),
    /// Read the history of one recorded execution.
    Get {
        run_id: String,
        #[arg(long)]
        project: Option<String>,
        /// Continue after this receipt ordinal, using the same cutoff.
        #[arg(long, requires = "cutoff")]
        after: Option<i64>,
        #[arg(long)]
        cutoff: Option<i64>,
    },
    /// Package recorded executions and their evidence into a new directory.
    Export(export::ExportArgs),
    /// Verify the files against an audit export manifest.
    VerifyExport { directory: PathBuf },
}

pub(super) async fn run(
    command: AuditCliCommand,
    root: &Path,
    loaded: &LoadedConfig,
    config: Option<&Path>,
) -> Result<()> {
    let client = audit_client(root, loaded, config);
    let value = match command {
        AuditCliCommand::Schema => {
            serde_json::to_value(schemars::schema_for!(AuditAction)).map_err(Into::into)
        }
        AuditCliCommand::Export(args) => export::run(args, &client, root, loaded, config).await,
        AuditCliCommand::VerifyExport { directory } => export::verify(&directory),
        command => client.send(command.into_action()?).await,
    }?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

impl AuditCliCommand {
    fn into_action(self) -> Result<AuditAction> {
        match self {
            Self::Append { file } => Ok(read::<AppendFile>(&file)?.into_action()),
            Self::Read { file } => {
                let action = read(&file)?;
                ensure!(
                    !matches!(action, AuditAction::Append { .. }),
                    "use audit append for writes"
                );
                Ok(action)
            }
            Self::List(args) => Ok(AuditAction::List {
                filter: args.filter(),
                project_id: args.project,
            }),
            Self::Get {
                run_id,
                project,
                after,
                cutoff,
            } => Ok(AuditAction::Get {
                run_id,
                after,
                cutoff,
                project_id: project,
            }),
            _ => unreachable!("this command is handled without an audit action"),
        }
    }
}

#[derive(Debug, clap::Args)]
pub(crate) struct FilterArgs {
    #[arg(long)]
    project: Option<String>,
    #[arg(long)]
    control: Option<String>,
    #[arg(long, requires = "control")]
    version: Option<i32>,
    /// Inclusive occurrence time in RFC 3339.
    #[arg(long)]
    from: Option<chrono::DateTime<chrono::Utc>>,
    /// Exclusive occurrence time in RFC 3339.
    #[arg(long)]
    until: Option<chrono::DateTime<chrono::Utc>>,
}

impl FilterArgs {
    fn filter(&self) -> RunFilter {
        RunFilter {
            control_key: self.control.clone(),
            version: self.version,
            from: self.from,
            until: self.until,
            ..RunFilter::default()
        }
    }
}

fn audit_client(
    root: &Path,
    loaded: &LoadedConfig,
    config: Option<&Path>,
) -> HistoryClient<AuditAction> {
    let context = CaptureContext::from_environment(root)
        .with_active_host(loaded)
        .hook;
    HistoryClient::new(
        loaded,
        config,
        context,
        ServiceRequest::Audit,
        "audit registry",
    )
}

fn read<T: DeserializeOwned>(file: &Path) -> Result<T> {
    let reader: Box<dyn Read> = if file == Path::new("-") {
        Box::new(io::stdin().lock())
    } else {
        Box::new(File::open(file).with_context(|| format!("open {}", file.display()))?)
    };
    let mut bytes = Vec::new();
    let limit = MAX_RECORD_BYTES * 2;
    reader.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "audit input exceeds {limit} bytes");
    serde_json::from_slice(&bytes).context("decode typed audit input")
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AppendFile {
    Bound(PreparedAuditRecord),
    Input(AuditInput),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedAuditRecord {
    project_id: String,
    record: AuditInput,
}

impl AppendFile {
    fn into_action(self) -> AuditAction {
        match self {
            Self::Bound(saved) => saved.into_action(),
            Self::Input(record) => AuditAction::Append {
                record: Box::new(record),
                expected_project: None,
            },
        }
    }
}

impl PreparedAuditRecord {
    fn into_action(self) -> AuditAction {
        AuditAction::Append {
            record: Box::new(self.record),
            expected_project: Some(self.project_id),
        }
    }
}

async fn send(client: &HistoryClient<AuditAction>, action: AuditAction) -> Result<AuditResponse> {
    serde_json::from_value(client.send(action).await?).context("decode audit response")
}
