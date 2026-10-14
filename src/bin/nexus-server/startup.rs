// @feature runtime
// @feature persistence
// @feature configuration
// @feature analyst
// @spec docs/features/runtime.md
// @spec docs/features/persistence.md
// @spec docs/features/configuration.md
// @spec docs/features/analyst.md
use super::{sqlite_import, Args};
use anyhow::{bail, Context, Result};
use nexus::runtime::server::{
    disable_in_process_model_providers, load_or_generate_secret, serve_listener, ServerSecret,
};
use nexus::{Config, NexusService};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

const HEALTH_READY_FILE_ENV: &str = "NEXUS_HEALTH_READY_FILE";

pub(super) async fn run(args: Args) -> Result<()> {
    PreparedServer::prepare(args).await?.serve().await
}

struct StartupInputs {
    listen: SocketAddr,
    database_url: String,
    secret: Option<String>,
    secret_file: Option<PathBuf>,
    allowed_origins: Vec<String>,
    config: Option<PathBuf>,
    import_sqlite: Option<PathBuf>,
}

impl StartupInputs {
    fn resolve(args: Args) -> Result<Self> {
        let database_url = args
            .database_url
            .or_else(|| std::env::var("NEXUS_DATABASE_URL").ok())
            .context("NEXUS_DATABASE_URL or --database-url is required")?;
        if database_url.trim().is_empty() {
            bail!("PostgreSQL database URL cannot be empty");
        }
        Ok(Self {
            listen: args.listen,
            database_url,
            secret: args.secret.or_else(|| std::env::var("NEXUS_SECRET").ok()),
            secret_file: args
                .secret_file
                .or_else(|| std::env::var_os("NEXUS_SECRET_FILE").map(PathBuf::from)),
            allowed_origins: args.allowed_origins,
            config: args.config,
            import_sqlite: args.import_sqlite,
        })
    }

    async fn bind(self) -> Result<PreparedServer> {
        let loaded_secret =
            load_or_generate_secret(self.secret.as_deref(), self.secret_file.as_deref())?;
        if loaded_secret.was_generated() {
            let path = self
                .secret_file
                .as_deref()
                .expect("generated server credentials always have a secret file");
            eprintln!("generated Nexus bearer secret at {}", path.display());
        }
        let loaded = Config::load(self.config.as_deref(), None)?;
        let service = Arc::new(NexusService::connect(loaded, &self.database_url)?);
        let listener = tokio::net::TcpListener::bind(self.listen)
            .await
            .with_context(|| format!("bind Nexus service at {}", self.listen))?;
        eprintln!("Nexus service listening on {}", listener.local_addr()?);
        Ok(PreparedServer {
            listener,
            service,
            secret: loaded_secret.verifier(),
            allowed_origins: self.allowed_origins,
        })
    }
}

struct PreparedServer {
    listener: tokio::net::TcpListener,
    service: Arc<NexusService>,
    secret: ServerSecret,
    allowed_origins: Vec<String>,
}

impl PreparedServer {
    async fn prepare(args: Args) -> Result<Self> {
        configure_provider_access(args.allow_local_providers);
        clear_health_readiness_marker()?;
        let inputs = StartupInputs::resolve(args)?;
        sqlite_import::run(inputs.import_sqlite.clone(), &inputs.database_url)?;
        inputs.bind().await
    }

    async fn serve(self) -> Result<()> {
        mark_health_ready()?;
        tokio::select! {
            result = serve_listener(
                self.listener,
                self.service,
                self.secret,
                self.allowed_origins,
            ) => result,
            result = termination_signal() => {
                result?;
                eprintln!("Nexus service received a termination signal");
                std::process::exit(0);
            }
        }
    }
}

fn configure_provider_access(allow_local_providers: bool) {
    if allow_local_providers {
        eprintln!(
            "WARNING: local model providers can read nexus-server files and process state; use only with external OS isolation"
        );
    } else {
        disable_in_process_model_providers();
    }
}

fn health_readiness_marker() -> Option<PathBuf> {
    std::env::var_os(HEALTH_READY_FILE_ENV).map(PathBuf::from)
}

fn clear_health_readiness_marker() -> Result<()> {
    let Some(path) = health_readiness_marker() else {
        return Ok(());
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("clear {}", path.display())),
    }
}

fn mark_health_ready() -> Result<()> {
    let Some(path) = health_readiness_marker() else {
        return Ok(());
    };
    std::fs::write(&path, format!("{}\n", std::process::id()))
        .with_context(|| format!("write {}", path.display()))
}

async fn termination_signal() -> Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("install SIGTERM handler")?;
    tokio::select! {
        _ = terminate.recv() => Ok(()),
        result = tokio::signal::ctrl_c() => result.context("install Ctrl-C handler"),
    }
}
