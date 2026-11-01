// @feature installation
// @feature runtime
// @feature observability-ui
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @spec docs/features/observability-ui.md
// @entrypoint run
use anyhow::{ensure, Result};
use clap::Args;
use std::io::Read;
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
    /// Configure the read-only web viewer's username, separate from agent credentials.
    #[arg(long, requires = "web_password_file")]
    web_user: Option<String>,
    /// Read the viewer password from a private file, or '-' for standard input.
    #[arg(long, requires = "web_user")]
    web_password_file: Option<PathBuf>,
    /// Store the salted viewer verifier at this private path.
    #[arg(long, requires = "web_user")]
    web_credentials_file: Option<PathBuf>,
}
impl SetupArgs {
    pub(super) fn run(self, loaded: &nexus::LoadedConfig, explicit: Option<&Path>) -> Result<()> {
        let password = self
            .web_password_file
            .as_deref()
            .map(read_viewer_password)
            .transpose()?;
        let executable = std::env::current_exe()?;
        let report = if self.refresh || self.web_user.is_some() {
            nexus::installation::refresh_machine(&executable, explicit)?
        } else {
            nexus::installation::setup_machine(&executable, explicit)?
        };
        let output = SetupReport {
            setup: report,
            update_management: nexus::updater::register_configured_service(loaded),
            web: configure_viewer(self.web_user, password, self.web_credentials_file)?,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        Ok(())
    }
}

fn configure_viewer(
    username: Option<String>,
    password: Option<Vec<u8>>,
    output: Option<PathBuf>,
) -> Result<Option<ViewerSetupReport>> {
    let (Some(username), Some(password)) = (username, password) else {
        return Ok(None);
    };
    let credentials_file =
        output.map_or_else(nexus::runtime::server::default_credentials_path, Ok)?;
    nexus::runtime::server::configure_credentials(&credentials_file, &username, &password)?;
    Ok(Some(ViewerSetupReport {
        username,
        credentials_file,
    }))
}

#[derive(serde::Serialize)]
struct SetupReport {
    #[serde(flatten)]
    setup: nexus::installation::MachineSetupReport,
    update_management: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    web: Option<ViewerSetupReport>,
}

#[derive(serde::Serialize)]
struct ViewerSetupReport {
    username: String,
    credentials_file: PathBuf,
}

fn read_viewer_password(path: &Path) -> Result<Vec<u8>> {
    if path != Path::new("-") {
        return nexus::runtime::server::read_password_file(path);
    }
    let mut password = Vec::new();
    std::io::stdin()
        .lock()
        .take(1027)
        .read_to_end(&mut password)?;
    if password.last() == Some(&b'\n') {
        password.pop();
        if password.last() == Some(&b'\r') {
            password.pop();
        }
    }
    ensure!(password.len() <= 1024, "viewer password exceeds 1024 bytes");
    Ok(password)
}
