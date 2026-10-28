// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
use serde::Serialize;
mod compose;
mod compose_evidence;
mod compose_format;
mod install;
mod operation;
mod registry;
mod release;
mod service;

#[derive(Debug, Clone, Serialize)]
pub struct BuildIdentity {
    pub version: String,
    pub revision: String,
    pub target: String,
    pub dirty: bool,
}
impl BuildIdentity {
    pub fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").into(),
            revision: env!("NEXUS_BUILD_REVISION").into(),
            target: env!("NEXUS_BUILD_TARGET").into(),
            dirty: env!("NEXUS_BUILD_DIRTY") == "true",
        }
    }
    fn display(&self) -> String {
        format!(
            "{} ({}; {}{})",
            self.version,
            self.revision,
            self.target,
            if self.dirty { "; dirty" } else { "" }
        )
    }
}
pub fn version() -> &'static str {
    env!("NEXUS_BUILD_VERSION")
}

#[derive(Debug, Default)]
pub struct Options {
    pub check: bool,
    pub force: bool,
    pub compose_file: Option<std::path::PathBuf>,
}
#[derive(Serialize)]
pub struct Report {
    pub status: &'static str,
    pub current: BuildIdentity,
    pub main_revision: String,
    pub available_version: Option<String>,
    pub service: &'static str,
    pub notes: Vec<String>,
}
// @entrypoint run
pub async fn run(
    loaded: &crate::LoadedConfig,
    explicit_config: Option<&std::path::Path>,
    options: &Options,
) -> anyhow::Result<Report> {
    let executable = std::env::current_exe()?;
    operation::execute(
        &registry::Registry::github()?,
        operation::UpdateContext {
            loaded,
            explicit_config,
            executable: &executable,
            current: BuildIdentity::current(),
        },
        options,
    )
    .await
}
pub fn register_compose(
    endpoint: &str,
    compose_file: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    operation::register(endpoint, compose_file)
}

fn state_directory() -> anyhow::Result<std::path::PathBuf> {
    use anyhow::Context;
    let path = dirs::data_local_dir()
        .context("user data directory is unavailable")?
        .join("nexus/updates");
    crate::private_state::directory_at(&path)?;
    Ok(path)
}

/// Register only proven local Compose ownership; other service modes stay explicit.
pub fn register_configured_service(loaded: &crate::LoadedConfig) -> &'static str {
    match &loaded.config.client {
        crate::config::ClientConfig::Local => "local_control",
        crate::config::ClientConfig::Remote(remote) => {
            let local = reqwest::Url::parse(&remote.endpoint)
                .ok()
                .is_some_and(|url| {
                    url.scheme() == "http"
                        && matches!(
                            url.host_str(),
                            Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
                        )
                });
            if local && register_compose(&remote.endpoint, None).is_ok() {
                "compose"
            } else {
                "external"
            }
        }
    }
}
