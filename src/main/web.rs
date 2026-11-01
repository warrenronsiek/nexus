// @feature observability-ui
// @feature runtime
// @spec docs/features/observability-ui.md
// @spec docs/features/runtime.md
// @entrypoint run
use anyhow::{ensure, Context, Result};
use clap::Args;
use nexus::runtime::server::{default_credentials_path, serve_browser_listener, BrowserConfig};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Debug, Args)]
pub(super) struct WebArgs {
    /// Bind the read-only viewer; keep loopback unless using an HTTPS reverse proxy.
    #[arg(long, default_value = "127.0.0.1:7338")]
    listen: SocketAddr,
    /// Private viewer credential file created by `nexus setup --web-user ...`.
    #[arg(long)]
    credentials_file: Option<PathBuf>,
    /// Exact HTTPS browser origin when publishing behind a reverse proxy.
    #[arg(long)]
    public_origin: Option<String>,
}

pub(super) async fn run(
    args: WebArgs,
    loaded: nexus::LoadedConfig,
    explicit: Option<&Path>,
) -> Result<()> {
    let credentials = args
        .credentials_file
        .map_or_else(default_credentials_path, Ok)?;
    ensure!(
        credentials.is_file(),
        "configure viewer credentials first with `nexus setup --web-user USER --web-password-file PATH`"
    );
    let browser = BrowserConfig::load(&credentials, args.public_origin.as_deref())?;
    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .context("bind Nexus web viewer")?;
    if matches!(loaded.config.client, nexus::config::ClientConfig::Local) {
        nexus::runtime::daemon::ensure_and_request(
            &loaded,
            explicit,
            &nexus::coordination::api::ServiceRequest::Status,
        )
        .await
        .context("start configured Nexus store for the web viewer")?;
    }
    eprintln!(
        "Nexus read-only viewer at http://{}",
        listener.local_addr()?
    );
    serve_browser_listener(listener, loaded, explicit.map(Path::to_path_buf), browser).await
}
