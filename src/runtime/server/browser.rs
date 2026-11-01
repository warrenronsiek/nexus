// @feature runtime
// @feature observability-ui
// @spec docs/features/runtime.md
// @spec docs/features/observability-ui.md
// @entrypoint serve_browser_listener
// @boundary browser-http: separate cookie authentication and closed reads never expose agent commands.
mod auth;
mod credentials;
mod origin;
mod read;
mod routes;
mod scope;
#[cfg(test)]
mod tests;

use crate::config::LoadedConfig;
use crate::coordination::api::ServiceRequest;
use crate::runtime::{client::NexusClient, dispatch};
use crate::NexusService;
use anyhow::Result;
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
pub use credentials::{configure_credentials, default_credentials_path, read_password_file};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

pub struct BrowserConfig {
    credentials: Option<credentials::Credentials>,
    origin: origin::BrowserOrigin,
}
impl BrowserConfig {
    pub fn load(path: &Path, public_origin: Option<&str>) -> Result<Self> {
        Ok(Self {
            credentials: credentials::Credentials::load(path)?,
            origin: origin::BrowserOrigin::parse(public_origin)?,
        })
    }
    pub fn disabled() -> Self {
        Self {
            credentials: None,
            origin: origin::BrowserOrigin::Loopback,
        }
    }
    pub fn configured(&self) -> bool {
        self.credentials.is_some()
    }
}

enum ReadDispatcher {
    Service {
        service: Arc<NexusService>,
        gate: dispatch::BlockingExecutionGate,
    },
    Client(Box<NexusClient>),
}
impl ReadDispatcher {
    async fn read(&self, request: ServiceRequest, project: Option<String>) -> Result<Vec<u8>> {
        match self {
            Self::Service { service, gate } => {
                let service = service.clone();
                let response = gate
                    .run(move || match project {
                        Some(project) => service.handle_remote(project, request),
                        None => service.handle(request),
                    })
                    .await
                    .map_err(|_| anyhow::anyhow!("browser read capacity unavailable"))?;
                Ok(serde_json::to_vec(&response)?)
            }
            Self::Client(client) => Ok(serde_json::to_vec(
                &client
                    .inspect_in_project(&request, project.as_deref())
                    .await?,
            )?),
        }
    }
}
struct BrowserState {
    auth: auth::Auth,
    origin: origin::BrowserOrigin,
    dispatcher: ReadDispatcher,
    project_scope: scope::ProjectScope,
    requests: Arc<Semaphore>,
}
fn router(
    config: BrowserConfig,
    dispatcher: ReadDispatcher,
    project_scope: scope::ProjectScope,
) -> Router {
    let state = Arc::new(BrowserState {
        auth: auth::Auth::new(config.credentials),
        origin: config.origin,
        dispatcher,
        project_scope,
        requests: Arc::new(Semaphore::new(16)),
    });
    Router::new()
        .route("/", get(routes::asset))
        .route("/ui", get(routes::asset))
        .route("/ui/{*path}", get(routes::asset))
        .route("/assets/{*path}", get(routes::asset))
        .route("/THIRD_PARTY_LICENSES.txt", get(routes::asset))
        .route("/ui/api/session", get(routes::session))
        .route("/ui/api/login", post(routes::login))
        .route("/ui/api/logout", post(routes::logout))
        .route("/ui/api/read", post(routes::read))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, routes::protect))
}
pub(super) fn service_router(
    config: BrowserConfig,
    service: Arc<NexusService>,
    gate: dispatch::BlockingExecutionGate,
) -> Router {
    router(
        config,
        ReadDispatcher::Service { service, gate },
        scope::ProjectScope::Global,
    )
}

/// A read-only local/proxy origin reuses the configured service rather than opening another store.
pub async fn serve_browser_listener(
    listener: TcpListener,
    loaded: LoadedConfig,
    explicit_config: Option<PathBuf>,
    browser: BrowserConfig,
) -> Result<()> {
    origin::validate_listener(listener.local_addr()?, &browser.origin)?;
    let project_scope = scope::ProjectScope::for_client(&loaded.config.client);
    let client = NexusClient::without_analytics(Arc::new(loaded), explicit_config);
    let router = router(
        browser,
        ReadDispatcher::Client(Box::new(client)),
        project_scope,
    );
    super::listener::serve(listener, router).await
}
const BODY_TIMEOUT: Duration = Duration::from_secs(5);
