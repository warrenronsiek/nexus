// @feature runtime
// @feature commit-review
// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/runtime.md
// @spec docs/features/commit-review.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @entrypoint main
use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use nexus::agents::integration::{self, Host};
use nexus::agents::reviewer;
use nexus::coordination::api::{
    AnalyzeCommand, ClaimQuery, ConflictQuery, EventQuery, MemoryAddCommand, MemoryContextQuery,
    MemoryReadScopeArg, MemoryScopeArg, MemorySearchQuery, MemoryStatusQuery, MemorySummaryQuery,
    ReleaseCommand, ResolveCommand, ServiceRequest, SessionQuery,
};
use nexus::coordination::domain::{ConflictScope, HookContext, RecordScope};
use nexus::installation;
use nexus::runtime::{daemon, hooks, mcp, script_exec};
use nexus::{Config, LoadedConfig};
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
    /// Execute a script while recording one privacy-limited usage observation.
    Exec {
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<OsString>,
    },
    /// Show daemon status and projection counts.
    Status,
    /// Show recent events.
    Events {
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show registered sessions.
    Sessions {
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
    /// Run parallel cross-model architecture and deletion review on the staged diff.
    ReviewCommit,
    /// Inspect or validate the resolved configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Check local configuration and storage prerequisites.
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

#[derive(Debug, Subcommand)]
enum IntegrationHost {
    Codex,
    Claude,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = std::env::current_dir()?;
    let explicit_config = cli.config;
    let loaded = Config::load(explicit_config.as_deref(), Some(&root))?;
    match cli.command {
        Command::Setup => {
            let report = installation::setup_machine(&std::env::current_exe()?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Command::Install => {
            let report = installation::install_repository(&root, explicit_config.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Command::Daemon => daemon::serve(loaded).await,
        Command::Mcp { agent } => {
            mcp::serve_stdio(loaded, explicit_config.as_deref(), &agent).await
        }
        Command::HookSessionEnd { agent } => {
            hooks::session_end(&loaded, explicit_config.as_deref(), &agent).await;
            Ok(())
        }
        Command::Exec { command } => {
            let code =
                script_exec::run(&loaded, explicit_config.as_deref(), &root, command).await?;
            std::process::exit(code);
        }
        Command::Memory { command } => run_memory_command(command, &root, &loaded).await,
        query @ (Command::Status
        | Command::Events { .. }
        | Command::Sessions { .. }
        | Command::Claims { .. }
        | Command::Conflicts { .. }) => run_query_command(query, &loaded).await,
        command => run_admin_command(command, &root, &loaded).await,
    }
}

async fn run_query_command(command: Command, loaded: &LoadedConfig) -> Result<()> {
    match command {
        Command::Status => {
            let result =
                daemon::request(&loaded.config.runtime.socket_path, &ServiceRequest::Status)
                    .await
                    .context("Nexus daemon is not running; start it with `nexus daemon`")?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::Events { project_id, limit } => {
            let result = daemon::request(
                &loaded.config.runtime.socket_path,
                &ServiceRequest::Events(EventQuery { project_id, limit }),
            )
            .await
            .context("Nexus daemon is not running")?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::Sessions { all } => {
            request_and_print(
                loaded,
                ServiceRequest::Sessions(SessionQuery {
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

async fn run_admin_command(command: Command, root: &Path, loaded: &LoadedConfig) -> Result<()> {
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
        Command::Doctor => {
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
        Command::Integrate { host } => {
            let host = match host {
                IntegrationHost::Codex => Host::Codex,
                IntegrationHost::Claude => Host::Claude,
            };
            let executable = std::env::current_exe()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&integration::instructions(host, &executable))?
            );
            Ok(())
        }
        _ => unreachable!("non-administrative command handled by main"),
    }
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

async fn request_and_print(loaded: &nexus::LoadedConfig, request: ServiceRequest) -> Result<()> {
    let result = daemon::request(&loaded.config.runtime.socket_path, &request)
        .await
        .context("Nexus daemon is not running")?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_cli_requires_write_scope_and_defaults_reads_to_layered() {
        let add = Cli::try_parse_from([
            "nexus",
            "memory",
            "add",
            "--scope",
            "project",
            "Prefer typed APIs",
        ])
        .unwrap();
        let Command::Memory {
            command: MemoryCommand::Add { scope, content },
        } = add.command
        else {
            panic!("expected memory add command");
        };
        assert_eq!(scope, MemoryScopeCli::Project);
        assert_eq!(content, "Prefer typed APIs");

        let context = Cli::try_parse_from(["nexus", "memory", "context"]).unwrap();
        let Command::Memory {
            command: MemoryCommand::Context { scope },
        } = context.command
        else {
            panic!("expected memory context command");
        };
        assert_eq!(scope, MemoryReadScopeCli::Layered);

        assert!(Cli::try_parse_from(["nexus", "memory", "add", "No scope"]).is_err());
    }
}
