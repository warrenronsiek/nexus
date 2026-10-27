// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary work-cli-json-and-source-context
use anyhow::{bail, Result};
use clap::{Args, Subcommand, ValueEnum};
use nexus::coordination::{
    api::{ServiceRequest, WorkCommand},
    domain::HookContext,
};
use nexus::runtime::client::NexusClient;
use nexus::work_history::*;
use nexus::LoadedConfig;
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[path = "work/capture.rs"]
mod capture;
#[path = "work/recording.rs"]
mod recording;
#[path = "work/transfer.rs"]
mod transfer;

#[derive(Debug, Args)]
pub(super) struct ExecArgs {
    #[arg(long)]
    pub kind: Option<String>,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub work_session: Option<String>,
    #[arg(long)]
    pub parent: Option<String>,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<OsString>,
}

impl ExecArgs {
    pub(super) async fn run(
        self,
        loaded: &LoadedConfig,
        explicit: Option<&Path>,
        root: &Path,
    ) -> Result<i32> {
        nexus::runtime::script_exec::run_with_options(
            loaded,
            explicit,
            root,
            self.command,
            nexus::runtime::script_exec::ExecOptions {
                kind: self.kind,
                name: self.name,
                work_session_id: self.work_session,
                parent_activity_id: self.parent,
            },
        )
        .await
    }
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub(super) enum Scope {
    Session,
    #[default]
    Activity,
}
impl Scope {
    fn subject(self, id: String) -> WorkSubject {
        match self {
            Self::Session => WorkSubject::Session { id },
            Self::Activity => WorkSubject::Activity { id },
        }
    }
}

#[derive(Debug, Subcommand)]
pub(super) enum WorkCliCommand {
    /// Print the captured identity handoff for an exact host session and optional turn.
    Context(capture::ContextArgs),
    /// Capture a native host or caller execution event from a bounded file.
    HostEvent(capture::HostEventArgs),
    /// Open a logical work session. The returned subject ID can be propagated to callers.
    Open {
        title: String,
        #[arg(long)]
        id: Option<String>,
    },
    /// Record an activity declaration and start, without executing it.
    Start(recording::StartArgs),
    /// Record the caller's outcome. Missing timing remains unknown.
    Finish(recording::FinishArgs),
    /// Record an observed milestone on a session or activity.
    Milestone {
        id: String,
        name: String,
        #[arg(long, value_enum, default_value_t = Scope::Session)]
        scope: Scope,
        #[arg(long)]
        environment: Option<String>,
    },
    /// Add a typed relationship; target is a WorkReference JSON object.
    Link {
        id: String,
        #[arg(long, value_enum, default_value_t = Scope::Activity)]
        scope: Scope,
        #[arg(long)]
        relation: String,
        #[arg(long)]
        target: String,
    },
    /// Record {format_version:1,observations:[...]}; - reads stdin.
    Record { file: PathBuf },
    /// Queue automatic metadata locally, then attempt delivery.
    #[command(hide = true)]
    Capture { file: PathBuf },
    #[command(hide = true)]
    GitHook { phase: String },
    /// List logical work sessions using receipt-bounded pages.
    List(ListArgs),
    /// Search concrete activities and retain links back to their work-session trees.
    Activities(ListArgs),
    /// Show a session or activity, observations, links, and capture gaps.
    Show {
        id: String,
        #[arg(long, value_enum, default_value_t = Scope::Activity)]
        scope: Scope,
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Page immediate children; expand a parent to navigate its descendants.
    Timeline {
        work_session_id: String,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 24)]
        limit: u32,
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Read immutable source observations for one subject.
    Events {
        id: String,
        #[arg(long, value_enum, default_value_t = Scope::Activity)]
        scope: Scope,
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 24)]
        limit: u32,
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Print the canonical versioned recording/query schema.
    Schema,
    /// Export project work history as checksummed JSONL, including unresolved observations.
    Export { output: PathBuf },
    /// Validate then replay a work-history export with original source provenance.
    Import { file: PathBuf },
    /// Show the local automatic-capture outbox health.
    Health,
    /// Attempt bounded delivery of queued automatic observations.
    Flush,
}

#[derive(Debug, Default, Args)]
pub(super) struct ListArgs {
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    repository: Option<String>,
    #[arg(long)]
    host_session_id: Option<String>,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    outcome: Option<String>,
    #[arg(long)]
    search: Option<String>,
    #[arg(long)]
    environment: Option<String>,
    #[arg(long)]
    occurred_after: Option<chrono::DateTime<chrono::Utc>>,
    #[arg(long)]
    occurred_before: Option<chrono::DateTime<chrono::Utc>>,
    #[arg(long)]
    after: Option<String>,
    #[arg(long, default_value_t = 24)]
    limit: u32,
}

struct WorkClient {
    client: NexusClient,
    context: HookContext,
}
impl WorkClient {
    async fn send(&self, command: WorkAction) -> Result<Value> {
        let response = self
            .client
            .ensure_and_request(&ServiceRequest::Work(WorkCommand {
                context: self.context.clone(),
                command,
            }))
            .await?;
        if response.get("ok").and_then(Value::as_bool) != Some(true) {
            bail!(
                "work history request failed: {}",
                response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("invalid response")
            );
        }
        Ok(response)
    }
}

pub(super) async fn run(
    command: WorkCliCommand,
    root: &Path,
    loaded: &LoadedConfig,
    explicit: Option<&Path>,
) -> Result<()> {
    let advisory = matches!(&command, WorkCliCommand::HostEvent(args) if args.advisory);
    let result = execute(command, loaded, explicit, root).await;
    if advisory {
        return Ok(());
    }
    let response = result?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

async fn execute(
    command: WorkCliCommand,
    loaded: &LoadedConfig,
    explicit: Option<&Path>,
    root: &Path,
) -> Result<Value> {
    match command {
        WorkCliCommand::Context(args) => capture::context(args, loaded),
        WorkCliCommand::HostEvent(args) => capture::host_event(args, loaded, explicit).await,
        WorkCliCommand::Schema => json_result(Ok(schemars::schema_for!(WorkAction))),
        WorkCliCommand::Export { output } => {
            transfer::export(&work_client(root, loaded, explicit), &output).await
        }
        WorkCliCommand::Import { file } => {
            transfer::import(&work_client(root, loaded, explicit), &file).await
        }
        WorkCliCommand::Health => json_result(nexus::runtime::work_capture::health(loaded)),
        WorkCliCommand::Flush => {
            json_result(nexus::runtime::work_capture::flush(loaded, explicit).await)
        }
        WorkCliCommand::Capture { file } => {
            nexus::runtime::work_capture::enqueue(
                loaded,
                &cli_context(root),
                recording::read_file(&file)?,
            )?;
            let _ = nexus::runtime::work_capture::flush(loaded, explicit).await;
            json_result(nexus::runtime::work_capture::health(loaded))
        }
        WorkCliCommand::GitHook { phase } => {
            nexus::runtime::work_capture::git_hook(loaded, root, &phase)?;
            let _ = nexus::runtime::work_capture::flush(loaded, explicit).await;
            json_result(nexus::runtime::work_capture::health(loaded))
        }
        command => send_action(&work_client(root, loaded, explicit), action(command)?).await,
    }
}

fn json_result<T: serde::Serialize>(value: Result<T>) -> Result<Value> {
    value.and_then(|value| serde_json::to_value(value).map_err(Into::into))
}

async fn send_action(client: &WorkClient, action: WorkAction) -> Result<Value> {
    let subject = match &action {
        WorkAction::Record { observations } => {
            observations.first().map(|event| event.subject.clone())
        }
        _ => None,
    };
    let mut response = client.send(action).await?;
    if let Some(subject) = subject {
        response["subject"] = serde_json::to_value(subject)?;
    }
    Ok(response)
}

fn action(command: WorkCliCommand) -> Result<WorkAction> {
    match command {
        WorkCliCommand::List(args) => args.action(Scope::Session),
        WorkCliCommand::Activities(args) => args.action(Scope::Activity),
        WorkCliCommand::Show {
            id,
            scope,
            project_id,
        } => Ok(WorkAction::Get {
            project_id,
            subject: scope.subject(id),
        }),
        WorkCliCommand::Timeline {
            work_session_id,
            parent,
            after,
            limit,
            project_id,
        } => Ok(WorkAction::Timeline {
            project_id,
            work_session_id,
            parent_activity_id: parent,
            after,
            limit: Some(limit),
        }),
        WorkCliCommand::Events {
            id,
            scope,
            after,
            limit,
            project_id,
        } => Ok(WorkAction::Events {
            project_id,
            subject: scope.subject(id),
            after,
            limit: Some(limit),
        }),
        command => recording::action(command),
    }
}

impl ListArgs {
    fn action(self, scope: Scope) -> Result<WorkAction> {
        let query = WorkQuery {
            repository: self.repository,
            host_session_id: self.host_session_id,
            kind: self.kind,
            outcome: self
                .outcome
                .map(|value| serde_json::from_value(Value::String(value)))
                .transpose()?,
            search: self.search,
            environment: self.environment,
            occurred_after: self.occurred_after,
            occurred_before: self.occurred_before,
            after: self.after,
            limit: Some(self.limit),
        };
        Ok(match scope {
            Scope::Session => WorkAction::List {
                project_id: self.project_id,
                query,
            },
            Scope::Activity => WorkAction::Activities {
                project_id: self.project_id,
                query,
            },
        })
    }
}

fn cli_context(root: &Path) -> HookContext {
    HookContext {
        session_id: std::env::var("NEXUS_HOST_SESSION_ID").unwrap_or_else(|_| "cli".into()),
        project_root: Some(root.to_string_lossy().into_owned()),
        agent: std::env::var("NEXUS_AGENT").unwrap_or_else(|_| "user".into()),
        turn_id: std::env::var("NEXUS_TURN_ID").ok(),
        model: std::env::var("NEXUS_MODEL").ok(),
    }
}

fn work_client(root: &Path, loaded: &LoadedConfig, explicit: Option<&Path>) -> WorkClient {
    WorkClient {
        client: NexusClient::new(loaded, explicit),
        context: cli_context(root),
    }
}
