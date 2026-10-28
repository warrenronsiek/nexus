// @feature code-concerns
// @feature runtime
// @spec docs/features/code-concerns.md
// @spec docs/features/runtime.md
// @boundary concern-cli-json: decode explicit caller records before sending typed commands
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use nexus::code_concerns::*;
use nexus::coordination::{api::ServiceRequest, domain::HookContext};
use nexus::LoadedConfig;
use std::path::{Path, PathBuf};

#[path = "concerns/recording.rs"]
mod recording;
#[path = "concerns/transfer.rs"]
mod transfer;

#[derive(Debug, Subcommand)]
pub(super) enum ConcernCliCommand {
    /// Report a code defect or evidenced design concern, with its first encounter.
    Report(recording::ReportArgs),
    /// Record encountering an existing concern again.
    Upvote(recording::UpvoteArgs),
    /// Append what was tried, why it was tried, and its reported outcome.
    Remedy(recording::RemedyArgs),
    /// Append a broader review hypothesis and its rationale.
    Review(recording::ReviewArgs),
    /// Add detail or evidence without claiming another occurrence.
    Evidence(recording::EvidenceArgs),
    /// Resolve with a reason and optional fixing location.
    Resolve(recording::ResolveArgs),
    /// Explicitly reopen a concern with a reason.
    Reopen(recording::ReopenArgs),
    /// Answer a duplicate proposal using the source reporter's private capability.
    Respond(recording::RespondArgs),
    /// Submit {format_version:1,observations:[...]}; - reads stdin.
    Record { file: PathBuf },
    /// List concerns, including resolved ones unless a status is selected.
    List(QueryArgs),
    /// Show problem-area recurrence totals and trends.
    Areas(QueryArgs),
    /// Read one concern and its recent history.
    Get {
        concern_id: String,
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Page immutable evidence for one concern.
    History {
        concern_id: String,
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Dump the complete current-project concern observation history.
    Dump {
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Export checksummed, reference-preserving concern history.
    Export { output: PathBuf },
    /// Verify and import a concern history export with original attribution.
    Import { file: PathBuf },
    /// Print the canonical recording and query schema.
    Schema,
}

#[derive(Debug, Default, Args)]
pub(super) struct QueryArgs {
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    repository: Option<String>,
    #[arg(long)]
    area_id: Option<String>,
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    search: Option<String>,
    #[arg(long)]
    occurred_after: Option<chrono::DateTime<chrono::Utc>>,
    #[arg(long)]
    occurred_before: Option<chrono::DateTime<chrono::Utc>>,
    /// recent, work_sessions, or returns.
    #[arg(long, default_value = "recent")]
    sort: String,
    #[arg(long)]
    after: Option<String>,
    #[arg(long)]
    limit: Option<u32>,
}

type ConcernClient = crate::history_client::HistoryClient<ConcernAction>;

pub(super) async fn run(
    command: ConcernCliCommand,
    root: &Path,
    loaded: &LoadedConfig,
    explicit: Option<&Path>,
) -> Result<()> {
    let client = ConcernClient::new(
        loaded,
        explicit,
        HookContext {
            session_id: std::env::var("NEXUS_HOST_SESSION_ID").unwrap_or_else(|_| "cli".into()),
            project_root: Some(root.to_string_lossy().into_owned()),
            agent: std::env::var("NEXUS_AGENT").unwrap_or_else(|_| "user".into()),
            turn_id: std::env::var("NEXUS_TURN_ID").ok(),
            model: std::env::var("NEXUS_MODEL").ok(),
        },
        ServiceRequest::Concerns,
        "code concern",
    );
    let response = match command {
        ConcernCliCommand::Schema => serde_json::to_value(schemars::schema_for!(ConcernAction))?,
        ConcernCliCommand::Export { output } => transfer::export(&client, &output).await?,
        ConcernCliCommand::Import { file } => transfer::import(&client, &file).await?,
        command => client.send(action(command)?).await?,
    };
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn action(command: ConcernCliCommand) -> Result<ConcernAction> {
    match command {
        ConcernCliCommand::List(args) => {
            let (project_id, query) = args.query()?;
            Ok(ConcernAction::List { project_id, query })
        }
        ConcernCliCommand::Areas(args) => {
            let (project_id, query) = args.query()?;
            Ok(ConcernAction::Dashboard { project_id, query })
        }
        ConcernCliCommand::Get {
            concern_id,
            project_id,
        } => Ok(ConcernAction::Get {
            concern_id,
            project_id,
        }),
        ConcernCliCommand::History {
            concern_id,
            project_id,
            after,
            limit,
        } => Ok(ConcernAction::History {
            concern_id,
            project_id,
            after,
            limit,
        }),
        ConcernCliCommand::Dump { project_id } => Ok(ConcernAction::Dump { project_id }),
        ConcernCliCommand::Record { file } => recording::read_bundle(&file),
        command => recording::action(command),
    }
}

impl QueryArgs {
    fn query(self) -> Result<(Option<String>, ConcernQuery)> {
        Ok((
            self.project_id,
            ConcernQuery {
                repository: self.repository,
                area_id: self.area_id,
                status: self.status.map(|v| recording::json_enum(&v)).transpose()?,
                search: self.search,
                occurred_after: self.occurred_after,
                occurred_before: self.occurred_before,
                sort: recording::json_enum(&self.sort)
                    .context("sort must be recent, work_sessions, or returns")?,
                after: self.after,
                limit: self.limit,
            },
        ))
    }
}
