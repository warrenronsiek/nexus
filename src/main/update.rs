// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @entrypoint run
use anyhow::Result;
use clap::Args;
use std::path::{Path, PathBuf};

#[derive(Debug, Args)]
pub(super) struct UpdateArgs {
    /// Compare the installed source revision with main without changing files or services.
    #[arg(long, conflicts_with = "record_compose")]
    check: bool,
    /// Replace a dirty, ahead, or unrelated source build with the verified main release.
    #[arg(long, conflicts_with = "record_compose")]
    force: bool,
    /// Prove which existing Compose source file owns the configured local service.
    #[arg(long)]
    compose_file: Option<PathBuf>,
    #[arg(long, hide = true, requires = "service_endpoint")]
    record_compose: Option<PathBuf>,
    #[arg(long, hide = true, requires = "record_compose")]
    service_endpoint: Option<String>,
}
pub(super) async fn run(
    args: UpdateArgs,
    loaded: &nexus::LoadedConfig,
    explicit: Option<&Path>,
) -> Result<()> {
    if let Some(file) = args.record_compose {
        nexus::updater::register_compose(args.service_endpoint.as_deref().unwrap(), Some(&file))?;
        println!("{{\"status\":\"managed_compose_recorded\"}}");
        return Ok(());
    }
    let options = nexus::updater::Options {
        check: args.check,
        force: args.force,
        compose_file: args.compose_file,
    };
    let report = nexus::updater::run(loaded, explicit, &options).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[derive(Debug, Args)]
pub(super) struct SetupArgs {
    /// Refresh installed resources even when no agent host is installed.
    #[arg(long, hide = true)]
    refresh: bool,
}
impl SetupArgs {
    pub(super) fn run(self, loaded: &nexus::LoadedConfig, explicit: Option<&Path>) -> Result<()> {
        let executable = std::env::current_exe()?;
        let report = if self.refresh {
            nexus::installation::refresh_machine(&executable, explicit)?
        } else {
            nexus::installation::setup_machine(&executable, explicit)?
        };
        let output = SetupReport {
            setup: report,
            update_management: nexus::updater::register_configured_service(loaded),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        Ok(())
    }
}

#[derive(serde::Serialize)]
struct SetupReport {
    #[serde(flatten)]
    setup: nexus::installation::MachineSetupReport,
    update_management: &'static str,
}
