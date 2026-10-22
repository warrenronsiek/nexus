// @feature evals
// @feature runtime
// @spec docs/features/evals.md
// @spec docs/features/runtime.md
// @boundary eval-cli-files-and-service-json
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use nexus::coordination::{
    api::{EvalAction, EvalCommand, ServiceRequest},
    domain::HookContext,
};
use nexus::evals::{EvalKind, EvalRecordInput};
use nexus::runtime::client::NexusClient;
use nexus::LoadedConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

#[path = "evals/artifacts.rs"]
mod artifacts;
#[path = "evals/transfer.rs"]
mod transfer;

const CHUNK_BYTES: usize = 256 * 1024;
const MAX_RECORD_FILE: u64 = 768 * 1024;

#[derive(Debug, Subcommand)]
pub(super) enum EvalCliCommand {
    /// Record a version-one JSON bundle. Use - for stdin.
    Record { file: PathBuf },
    /// List recorded history without fetching artifact bodies.
    List(EvalListArgs),
    /// Show one immutable record, its links and capture provenance.
    Get {
        id: String,
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Trace the recorded motivation and upstream evidence.
    Why {
        id: String,
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long, default_value_t = 16)]
        limit: u32,
    },
    /// Print the canonical recording/query JSON Schema.
    Schema,
    /// Upload new bytes absent from Git, deduplicated by SHA-256.
    Upload { file: PathBuf },
    /// Discard an unfinished upload so incorrect chunks can be retried.
    AbortUpload { digest: String },
    /// Download a verified stored blob. Git artifacts stay Git references.
    Download { digest: String, output: PathBuf },
    /// Stream current-project history with Git references and stored blobs.
    Export { output: PathBuf },
    /// Import an exported reference bundle; never fetch or execute Git content.
    Import { file: PathBuf },
}

#[derive(Debug, Default, Args)]
pub(super) struct EvalListArgs {
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    repository: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    search: Option<String>,
    #[arg(long)]
    after: Option<String>,
    #[arg(long, default_value_t = 24)]
    limit: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordFile {
    format_version: u32,
    records: Vec<EvalRecordInput>,
}

struct EvalClient<'a> {
    client: NexusClient,
    root: &'a Path,
}

impl<'a> EvalClient<'a> {
    fn new(root: &'a Path, loaded: &LoadedConfig) -> Self {
        Self {
            client: NexusClient::new(loaded, None),
            root,
        }
    }

    async fn send(&self, command: EvalAction) -> Result<Value> {
        let context = HookContext {
            session_id: "cli".into(),
            project_root: Some(self.root.to_string_lossy().into_owned()),
            agent: "user".into(),
            turn_id: None,
            model: None,
        };
        let response = self
            .client
            .request(&ServiceRequest::Evals(EvalCommand { context, command }))
            .await?;
        if response.get("ok").and_then(Value::as_bool) != Some(true) {
            bail!(
                "eval recording/query failed: {}",
                response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("invalid response")
            );
        }
        Ok(response)
    }

    async fn get(&self, id: &str) -> Result<nexus::evals::EvalRecord> {
        let response = self
            .send(EvalAction::Get {
                id: id.into(),
                project_id: None,
            })
            .await?;
        serde_json::from_value(response["record"].clone())
            .with_context(|| format!("missing or malformed eval record {id}"))
    }
}

pub(super) async fn run(command: EvalCliCommand, root: &Path, loaded: &LoadedConfig) -> Result<()> {
    if let Some(response) = execute(&EvalClient::new(root, loaded), command).await? {
        println!("{}", serde_json::to_string_pretty(&response)?);
    }
    Ok(())
}

async fn execute(client: &EvalClient<'_>, command: EvalCliCommand) -> Result<Option<Value>> {
    match command {
        EvalCliCommand::Schema => serde_json::to_value(schemars::schema_for!(EvalAction))
            .map(Some)
            .map_err(Into::into),
        command @ (EvalCliCommand::Record { .. }
        | EvalCliCommand::List(_)
        | EvalCliCommand::Get { .. }
        | EvalCliCommand::Why { .. }) => client.send(request_action(command)?).await.map(Some),
        EvalCliCommand::Upload { file } => client.upload(&file).await.map(Some),
        EvalCliCommand::AbortUpload { digest } => client
            .send(EvalAction::BlobAbort { digest })
            .await
            .map(Some),
        EvalCliCommand::Download { digest, output } => {
            client.download(&digest, &output).await.map(|()| None)
        }
        EvalCliCommand::Export { output } => transfer::export(client, &output).await.map(|()| None),
        EvalCliCommand::Import { file } => transfer::import(client, &file).await.map(Some),
    }
}

fn request_action(command: EvalCliCommand) -> Result<EvalAction> {
    let action = match command {
        EvalCliCommand::Record { file } => EvalAction::Record {
            records: read_record_file(&file)?,
        },
        EvalCliCommand::List(args) => EvalAction::List {
            project_id: args.project_id,
            kind: args
                .kind
                .map(|s| serde_json::from_value::<EvalKind>(Value::String(s)))
                .transpose()
                .context("unknown eval kind")?,
            repository: args.repository,
            model: args.model,
            search: args.search,
            after: args.after,
            limit: Some(args.limit),
        },
        EvalCliCommand::Get { id, project_id } => EvalAction::Get { id, project_id },
        EvalCliCommand::Why {
            id,
            project_id,
            limit,
        } => EvalAction::Why {
            id,
            project_id,
            limit: Some(limit),
        },
        _ => unreachable!("record/query commands classified before conversion"),
    };
    Ok(action)
}

fn read_record_file(file: &Path) -> Result<Vec<EvalRecordInput>> {
    let bytes = read_record_bytes(file)?;
    let bundle: RecordFile =
        serde_json::from_slice(&bytes).context("expected {format_version:1,records:[...]}")?;
    anyhow::ensure!(
        bundle.format_version == 1,
        "unsupported eval record file version"
    );
    Ok(bundle.records)
}

fn read_record_bytes(file: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let input: Box<dyn Read> = if file == Path::new("-") {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(file)?)
    };
    input.take(MAX_RECORD_FILE + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_RECORD_FILE,
        "eval record file exceeds 768 KiB; use streamed import for history"
    );
    Ok(bytes)
}
