// @feature runtime
// @feature persistence
// @feature configuration
// @feature analyst
// @spec docs/features/runtime.md
// @spec docs/features/persistence.md
// @spec docs/features/configuration.md
// @spec docs/features/analyst.md
// @entrypoint main
use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::net::SocketAddr;
use std::path::PathBuf;

#[path = "nexus-server/sqlite_import.rs"]
mod sqlite_import;
#[path = "nexus-server/startup.rs"]
mod startup;

#[derive(Debug, Parser)]
#[command(
    name = "nexus-server",
    version,
    about = "Authenticated Nexus HTTP and MCP service"
)]
struct Args {
    /// Import exits after the atomic database transfer without starting schedulers or a listener.
    #[arg(long, value_enum, default_value = "serve")]
    mode: ServerMode,
    /// Address and port exposed by the service.
    #[arg(long, default_value = "127.0.0.1:7337")]
    listen: SocketAddr,
    /// PostgreSQL URL. Prefer NEXUS_DATABASE_URL so credentials stay out of process listings.
    #[arg(long)]
    database_url: Option<String>,
    /// Bearer secret. Prefer NEXUS_SECRET or --secret-file outside local development.
    #[arg(long)]
    secret: Option<String>,
    /// Read or atomically create a mode-0600 bearer secret at this path.
    #[arg(long)]
    secret_file: Option<PathBuf>,
    /// Accept this Origin header on authenticated HTTP requests.
    #[arg(long = "allow-origin")]
    allowed_origins: Vec<String>,
    /// Load non-secret service configuration from this TOML file.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Import an existing Nexus SQLite database before accepting requests.
    #[arg(long, required_if_eq("mode", "import"))]
    import_sqlite: Option<PathBuf>,
    /// Permit same-UID provider subprocesses. Requires external OS isolation from service secrets.
    #[arg(long)]
    allow_local_providers: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ServerMode {
    Serve,
    Import,
    Export,
}

#[tokio::main]
async fn main() -> Result<()> {
    startup::run(Args::parse()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_provider_execution_requires_an_explicit_server_flag() {
        assert!(
            !Args::try_parse_from(["nexus-server"])
                .unwrap()
                .allow_local_providers
        );
        assert!(
            Args::try_parse_from(["nexus-server", "--allow-local-providers"])
                .unwrap()
                .allow_local_providers
        );
    }

    #[test]
    fn sqlite_import_is_an_explicit_server_startup_option() {
        let args = Args::try_parse_from([
            "nexus-server",
            "--import-sqlite",
            "/var/lib/nexus-import/nexus.db",
        ])
        .unwrap();
        assert_eq!(
            args.import_sqlite,
            Some(PathBuf::from("/var/lib/nexus-import/nexus.db"))
        );
    }

    #[test]
    fn offline_import_requires_an_explicit_source_and_never_selects_serve_mode() {
        assert!(Args::try_parse_from(["nexus-server", "--mode", "import"]).is_err());
        let args = Args::try_parse_from([
            "nexus-server",
            "--mode",
            "import",
            "--import-sqlite",
            "/snapshot.db",
        ])
        .unwrap();
        assert_eq!(args.mode, ServerMode::Import);
    }

    #[test]
    fn offline_export_selects_a_stream_without_requiring_an_import_source() {
        let args = Args::try_parse_from(["nexus-server", "--mode", "export"]).unwrap();
        assert_eq!(args.mode, ServerMode::Export);
    }
}
