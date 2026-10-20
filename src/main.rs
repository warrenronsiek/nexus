// @feature runtime
// @feature commit-review
// @feature usage-analytics
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/runtime.md
// @spec docs/features/commit-review.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
// @entrypoint main
// @boundary cli-and-json
#[path = "main/sos.rs"]
mod sos_cli;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use nexus::agents::integration::{self, Host};
use nexus::agents::reviewer;
use nexus::coordination::api::{
    AnalyzeCommand, ClaimQuery, ConflictQuery, EventQuery, MemoryAddCommand, MemoryContextQuery,
    MemoryReadScopeArg, MemoryScopeArg, MemorySearchQuery, MemoryStatusQuery, MemorySummaryQuery,
    PapercutCommand, PapercutDumpScope, ReleaseCommand, ResolveCommand, ServiceRequest,
    SessionQuery,
};
use nexus::coordination::domain::{ConflictScope, HookContext, RecordScope};
use nexus::installation;
use nexus::papercuts::PapercutReviewDecision;
use nexus::runtime::{client::NexusClient, daemon, hooks, mcp, script_exec, terminal};
use nexus::{Config, LoadedConfig};
use sos_cli::SosCliCommand;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "nexus",
    version,
    about = "Advisory coordination for coding agents"
)]
struct Cli {
    /// Load user configuration from this TOML file.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Configure installed agent hosts and bundled review skills on this machine.
    Setup,
    /// Install Nexus coordination into the current Git repository.
    Install,
    /// Run the local coordination daemon in the foreground.
    Daemon,
    /// Run the MCP server over stdio, starting the daemon when necessary.
    Mcp {
        /// Attribute simple caller-authored memories to this host process.
        #[arg(long, default_value = "mcp")]
        agent: String,
    },
    /// Receive a host SessionEnd hook over stdin and release its claims.
    #[command(hide = true)]
    HookSessionEnd {
        #[arg(long)]
        agent: String,
    },
    /// Open the bundled interactive terminal dashboard.
    Tui,
    /// Execute a script while recording one privacy-limited usage observation.
    Exec {
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<OsString>,
    },
    /// Show service status and projection counts.
    Status,
    /// Show recent events.
    Events {
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long, default_value_t = 64)]
        limit: usize,
    },
    /// Show registered sessions.
    Sessions {
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// Show advisory path claims.
    Claims {
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// Show detected overlaps.
    Conflicts {
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// Record an optional resolution for an overlap.
    Resolve {
        conflict_id: String,
        #[arg(long, default_value = "acknowledged")]
        resolution: String,
    },
    /// Release advisory claims owned by a session.
    Release {
        session_id: String,
        #[arg(long)]
        path: Option<String>,
    },
    /// Ask the explicitly configured read-only analyst to assess a conflict.
    Analyze { conflict_id: String },
    /// Record, inspect, and maintain durable agent memories.
    Memory {
        #[command(subcommand)]
        command: MemoryCommand,
    },
    /// Record, inspect, and maintain development-environment papercuts.
    Papercuts {
        #[command(subcommand)]
        command: PapercutsCommand,
    },
    /// Report, inspect, and maintain complex problems that need help.
    Sos {
        #[command(subcommand)]
        command: SosCliCommand,
    },
    /// Run parallel cross-model architecture and deletion review on the staged diff.
    ReviewCommit,
    /// Inspect or validate the resolved configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Check configured transport and storage prerequisites.
    Doctor,
    /// Print host registration and hook configuration.
    Integrate {
        #[command(subcommand)]
        host: IntegrationHost,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Show,
    Check,
}

#[derive(Debug, Subcommand)]
enum MemoryCommand {
    /// Show memory counts, pending consolidation, and provider health.
    Status,
    /// Print bounded context as Nexus would inject it into a new session.
    Context {
        #[arg(long, value_enum, default_value_t = MemoryReadScopeCli::Layered)]
        scope: MemoryReadScopeCli,
    },
    /// Record one explicit durable memory.
    Add {
        #[arg(long, value_enum)]
        scope: MemoryScopeCli,
        content: String,
    },
    /// Search authoritative raw memories with a case-insensitive Rust regex.
    Search {
        #[arg(long, value_enum, default_value_t = MemoryReadScopeCli::Layered)]
        scope: MemoryReadScopeCli,
        regex: String,
    },
    /// Expand a derived summary into its two source children.
    Expand { summary_id: String },
    /// Invalidate a derived summary and its stored ancestors.
    Invalidate { summary_id: String },
    /// Request one immediate background-consolidation batch.
    Consolidate,
}

#[derive(Debug, Subcommand)]
enum PapercutsCommand {
    /// List the open canonical papercuts for the current project.
    List,
    /// Dump the complete, unbounded papercut registry.
    Dump {
        /// Include every observed project instead of only the current project.
        #[arg(long)]
        all_projects: bool,
    },
    /// Report a development-environment papercut.
    Report {
        title: String,
        #[arg(long, default_value = "")]
        description: String,
    },
    /// Record another encounter with an existing papercut.
    Upvote {
        papercut_id: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Mark a solved papercut resolved while retaining its history.
    Remove {
        papercut_id: String,
        #[arg(long)]
        resolution: String,
    },
    /// Respond to a proposed duplicate review.
    Respond {
        review_id: String,
        #[arg(long)]
        reporter_token: String,
        #[arg(long, value_enum)]
        decision: PapercutReviewDecisionCli,
        #[arg(long)]
        response: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MemoryScopeCli {
    Global,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MemoryReadScopeCli {
    Layered,
    Global,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum PapercutReviewDecisionCli {
    Approve,
    Reject,
}

#[derive(Debug, Subcommand)]
enum IntegrationHost {
    Codex,
    Claude,
}

struct CliInvocation {
    command: Command,
    root: PathBuf,
    explicit_config: Option<PathBuf>,
    loaded: LoadedConfig,
}

impl CliInvocation {
    fn resolve(cli: Cli) -> Result<Self> {
        let root = std::env::current_dir()?;
        let explicit_config = cli
            .config
            .or_else(|| std::env::var_os("NEXUS_CONFIG").map(PathBuf::from))
            .map(|path| root.join(path));
        let loaded = Config::load(explicit_config.as_deref(), Some(&root))?;
        Ok(Self {
            command: cli.command,
            root,
            explicit_config,
            loaded,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let CliInvocation {
        command,
        root,
        explicit_config,
        loaded,
    } = CliInvocation::resolve(Cli::parse())?;
    match command {
        Command::Setup => {
            let report =
                installation::setup_machine(&std::env::current_exe()?, explicit_config.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Command::Install => {
            let report = installation::install_repository(&root, explicit_config.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Command::Daemon => {
            if matches!(
                &loaded.config.client,
                nexus::config::ClientConfig::Remote(_)
            ) {
                bail!("a remote Nexus service is configured; the local daemon is disabled");
            }
            daemon::serve(loaded).await
        }
        Command::Mcp { agent } => {
            mcp::serve_stdio(loaded, explicit_config.as_deref(), &agent).await
        }
        Command::HookSessionEnd { agent } => {
            hooks::session_end(&loaded, explicit_config.as_deref(), &agent).await;
            Ok(())
        }
        Command::Tui => terminal::open(explicit_config.as_deref(), &root).await,
        Command::Exec { command } => {
            let code =
                script_exec::run(&loaded, explicit_config.as_deref(), &root, command).await?;
            std::process::exit(code);
        }
        Command::Memory { command } => run_memory_command(command, &root, &loaded).await,
        Command::Papercuts { command } => run_papercuts_command(command, &root, &loaded).await,
        Command::Sos { command } => sos_cli::run(command, &root, &loaded).await,
        query @ (Command::Status
        | Command::Events { .. }
        | Command::Sessions { .. }
        | Command::Claims { .. }
        | Command::Conflicts { .. }) => run_query_command(query, &loaded).await,
        command => run_admin_command(command, &root, &loaded, explicit_config.as_deref()).await,
    }
}

async fn run_papercuts_command(
    command: PapercutsCommand,
    root: &Path,
    loaded: &LoadedConfig,
) -> Result<()> {
    let context = HookContext {
        session_id: "cli".into(),
        project_root: Some(root.to_string_lossy().into_owned()),
        agent: "user".into(),
        turn_id: None,
        model: None,
    };
    let command = match command {
        PapercutsCommand::List => PapercutCommand::List {
            context,
            project_id: None,
        },
        PapercutsCommand::Dump { all_projects } => PapercutCommand::Dump {
            context,
            scope: if all_projects {
                PapercutDumpScope::All
            } else {
                PapercutDumpScope::Project
            },
        },
        PapercutsCommand::Report { title, description } => PapercutCommand::Report {
            context,
            title,
            description,
        },
        PapercutsCommand::Upvote { papercut_id, note } => PapercutCommand::Upvote {
            context,
            papercut_id,
            note,
        },
        PapercutsCommand::Remove {
            papercut_id,
            resolution,
        } => PapercutCommand::Remove {
            context,
            papercut_id,
            resolution,
        },
        PapercutsCommand::Respond {
            review_id,
            reporter_token,
            decision,
            response,
        } => PapercutCommand::Respond {
            context,
            review_id,
            reporter_token,
            decision: decision.into(),
            response,
        },
    };
    request_and_print(loaded, ServiceRequest::Papercuts(command)).await
}

async fn run_query_command(command: Command, loaded: &LoadedConfig) -> Result<()> {
    match command {
        Command::Status => {
            let result = NexusClient::new(loaded, None)
                .request(&ServiceRequest::Status)
                .await
                .context("Nexus is not reachable")?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::Events { project_id, limit } => {
            let result = NexusClient::new(loaded, None)
                .request(&ServiceRequest::Events(EventQuery { project_id, limit }))
                .await
                .context("Nexus is not reachable")?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::Sessions { project_id, all } => {
            request_and_print(
                loaded,
                ServiceRequest::Sessions(SessionQuery {
                    project_id,
                    scope: if all {
                        RecordScope::All
                    } else {
                        RecordScope::Active
                    },
                }),
            )
            .await
        }
        Command::Claims { project_id, all } => {
            request_and_print(
                loaded,
                ServiceRequest::Claims(ClaimQuery {
                    project_id,
                    scope: if all {
                        RecordScope::All
                    } else {
                        RecordScope::Active
                    },
                }),
            )
            .await
        }
        Command::Conflicts { project_id, all } => {
            request_and_print(
                loaded,
                ServiceRequest::Conflicts(ConflictQuery {
                    project_id,
                    scope: if all {
                        ConflictScope::All
                    } else {
                        ConflictScope::Open
                    },
                }),
            )
            .await
        }
        _ => unreachable!("non-query command handled by main"),
    }
}

async fn run_admin_command(
    command: Command,
    root: &Path,
    loaded: &LoadedConfig,
    explicit_config: Option<&Path>,
) -> Result<()> {
    match command {
        Command::Resolve {
            conflict_id,
            resolution,
        } => {
            request_and_print(
                loaded,
                ServiceRequest::Resolve(ResolveCommand {
                    conflict_id,
                    resolution,
                }),
            )
            .await
        }
        Command::Release { session_id, path } => {
            request_and_print(
                loaded,
                ServiceRequest::Release(ReleaseCommand { session_id, path }),
            )
            .await
        }
        Command::Analyze { conflict_id } => {
            request_and_print(
                loaded,
                ServiceRequest::Analyze(AnalyzeCommand { conflict_id }),
            )
            .await
        }
        Command::ReviewCommit => {
            let report = reviewer::review_staged_commit(&loaded.config.review, root)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Command::Config { command } => {
            match command {
                ConfigCommand::Show => println!("{}", serde_json::to_string_pretty(&loaded)?),
                ConfigCommand::Check => println!("configuration is valid ({})", loaded.hash),
            }
            Ok(())
        }
        Command::Doctor => run_doctor(loaded).await,
        Command::Integrate { host } => print_integration(host, explicit_config),
        _ => unreachable!("non-administrative command handled by main"),
    }
}

async fn run_doctor(loaded: &LoadedConfig) -> Result<()> {
    if matches!(
        &loaded.config.client,
        nexus::config::ClientConfig::Remote(_)
    ) {
        let status = NexusClient::new(loaded, None)
            .request(&ServiceRequest::Status)
            .await
            .context("connect to configured remote Nexus service")?;
        if status.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
            bail!("remote Nexus service is not ready");
        }
        println!("configuration: valid\nservice: ready\ntransport: remote\nmode: advisory-only");
        return Ok(());
    }
    if let Some(parent) = loaded.config.storage.database_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _store = nexus::persistence::Store::open(&loaded.config.storage.database_path)?;
    println!(
        "configuration: valid\ndatabase: ready\nsocket: {}\nmode: advisory-only",
        loaded.config.runtime.socket_path.display()
    );
    Ok(())
}

fn print_integration(host: IntegrationHost, explicit_config: Option<&Path>) -> Result<()> {
    let host = match host {
        IntegrationHost::Codex => Host::Codex,
        IntegrationHost::Claude => Host::Claude,
    };
    let executable = std::env::current_exe()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&integration::instructions(
            host,
            &executable,
            explicit_config,
        ))?
    );
    Ok(())
}

async fn run_memory_command(
    command: MemoryCommand,
    root: &Path,
    loaded: &LoadedConfig,
) -> Result<()> {
    let project_root = Some(root.to_string_lossy().into_owned());
    let request = match command {
        MemoryCommand::Status => ServiceRequest::MemoryStatus(MemoryStatusQuery { project_root }),
        MemoryCommand::Context { scope } => ServiceRequest::MemoryContext(MemoryContextQuery {
            scope: scope.into(),
            project_root,
        }),
        MemoryCommand::Add { scope, content } => ServiceRequest::MemoryAdd(MemoryAddCommand {
            context: HookContext {
                session_id: "cli".into(),
                project_root,
                agent: "user".into(),
                turn_id: None,
                model: None,
            },
            scope: scope.into(),
            content,
        }),
        MemoryCommand::Search { scope, regex } => ServiceRequest::MemorySearch(MemorySearchQuery {
            scope: scope.into(),
            project_root,
            regex,
            limit: 50,
        }),
        MemoryCommand::Expand { summary_id } => {
            ServiceRequest::MemoryExpand(MemorySummaryQuery { summary_id })
        }
        MemoryCommand::Invalidate { summary_id } => {
            ServiceRequest::MemoryInvalidate(MemorySummaryQuery { summary_id })
        }
        MemoryCommand::Consolidate => ServiceRequest::MemoryConsolidate,
    };
    request_and_print(loaded, request).await
}

impl From<MemoryScopeCli> for MemoryScopeArg {
    fn from(scope: MemoryScopeCli) -> Self {
        match scope {
            MemoryScopeCli::Global => Self::Global,
            MemoryScopeCli::Project => Self::Project,
        }
    }
}

impl From<MemoryReadScopeCli> for MemoryReadScopeArg {
    fn from(scope: MemoryReadScopeCli) -> Self {
        match scope {
            MemoryReadScopeCli::Layered => Self::Layered,
            MemoryReadScopeCli::Global => Self::Global,
            MemoryReadScopeCli::Project => Self::Project,
        }
    }
}

impl From<PapercutReviewDecisionCli> for PapercutReviewDecision {
    fn from(decision: PapercutReviewDecisionCli) -> Self {
        match decision {
            PapercutReviewDecisionCli::Approve => Self::Approve,
            PapercutReviewDecisionCli::Reject => Self::Reject,
        }
    }
}

async fn request_and_print(loaded: &nexus::LoadedConfig, request: ServiceRequest) -> Result<()> {
    let result = NexusClient::new(loaded, None)
        .request(&request)
        .await
        .context("Nexus is not reachable")?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

#[cfg(test)]
#[path = "main/tests.rs"]
mod tests;
