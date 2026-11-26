// @feature audit-registry
// @feature runtime
// @spec docs/features/audit-registry.md
// @spec docs/features/runtime.md
// @feature agent-traces
// @spec docs/features/agent-traces.md
use super::{
    audit_cli, run_admin_command, run_memory_command, run_papercuts_command, run_query_command,
    sos_cli, traces_cli, Command,
};
use anyhow::Result;
use nexus::LoadedConfig;
use std::path::Path;

pub(super) async fn run(
    command: Command,
    root: &Path,
    loaded: &LoadedConfig,
    config: Option<&Path>,
) -> Result<()> {
    match command {
        Command::Memory { command } => run_memory_command(command, root, loaded).await,
        Command::Papercuts { command } => run_papercuts_command(command, root, loaded).await,
        Command::Sos { command } => sos_cli::run(command, root, loaded).await,
        Command::Traces(command) => traces_cli::run(command, root, loaded, config).await,
        query @ (Command::Status
        | Command::Events { .. }
        | Command::Sessions { .. }
        | Command::Claims { .. }
        | Command::Conflicts { .. }) => run_query_command(query, loaded).await,
        Command::Audit(command) => audit_cli::run(command, root, loaded, config).await,
        Command::ReviewCommit(args) => args.run(root, loaded, config).await,
        command => run_admin_command(command, root, loaded, config).await,
    }
}
