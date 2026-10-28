// @feature runtime
// @feature coordination
// @feature configuration
// @spec docs/features/runtime.md
// @spec docs/features/coordination.md
// @spec docs/features/configuration.md
// @boundary native-execution-directory: resolve literal host working directories locally without interpreting shell or orchestration code.
use super::git_intents::LiteralGit;
use crate::config::{Config, LoadedConfig};
use crate::coordination::api::ServiceRequest;
use crate::coordination::api::ToolHookPhase;
use crate::coordination::domain::ToolHookInput;
use crate::coordination::shell_invocation;
use crate::runtime::client::NexusClient;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;

pub(super) struct CheckoutClients {
    base_root: Option<PathBuf>,
    base: Arc<LoadedConfig>,
    explicit: Option<PathBuf>,
    clients: Mutex<HashMap<String, Arc<NexusClient>>>,
    slots: Arc<Semaphore>,
}

impl CheckoutClients {
    pub(super) fn new(loaded: &LoadedConfig, explicit: Option<PathBuf>) -> Self {
        Self {
            base_root: std::env::current_dir()
                .ok()
                .map(|cwd| repository_root(&cwd).unwrap_or(cwd)),
            base: Arc::new(loaded.clone()),
            explicit,
            clients: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(32)),
        }
    }

    pub(super) async fn resolve(
        &self,
        request: ServiceRequest,
        base: Arc<NexusClient>,
    ) -> Result<(ServiceRequest, Arc<NexusClient>)> {
        let (input, phase) = match request {
            ServiceRequest::ToolHook { input, phase } if input.context.project_root.is_some() => {
                (input, phase)
            }
            other => return Ok((other, base)),
        };
        let (request, loaded) = self.prepare(input, phase).await?;
        if loaded.hash == self.base.hash {
            return Ok((request, base));
        }
        let mut clients = self
            .clients
            .lock()
            .map_err(|_| anyhow::anyhow!("checkout client lock poisoned"))?;
        if let Some(client) = clients.get(&loaded.hash) {
            return Ok((request, client.clone()));
        }
        if clients.len() >= 64 {
            bail!("coordination checkout configuration limit reached");
        }
        let client = Arc::new(NexusClient::new(&loaded, self.explicit.as_deref()));
        clients.insert(loaded.hash, client.clone());
        Ok((request, client))
    }
    pub(super) async fn stop(
        &self,
        context: &crate::coordination::domain::HookContext,
    ) -> serde_json::Value {
        crate::runtime::hooks::stop_visited(&self.base, self.explicit.as_deref(), context).await
    }
    async fn prepare(
        &self,
        mut input: ToolHookInput,
        phase: ToolHookPhase,
    ) -> Result<(ServiceRequest, LoadedConfig)> {
        let explicit = self.explicit.clone();
        let base_config = self.base.clone();
        let base_root = self.base_root.clone();
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .context("checkout resolution is busy")?;
        let worker = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let loaded = prepare_checkout(&mut input, phase, explicit.as_deref())?;
            let additional_checkout = loaded.hash != base_config.hash
                || input.context.project_root.as_deref().map(Path::new) != base_root.as_deref();
            if additional_checkout
                && crate::runtime::checkout_visits::remember(&base_config, &loaded, &input.context)
                    .is_err()
            {
                eprintln!("Nexus checkout teardown metadata unavailable; failed shutdown delivery relies on claim expiry");
            }
            Ok::<_, anyhow::Error>((ServiceRequest::ToolHook { input, phase }, loaded))
        });
        let resolved = tokio::time::timeout(Duration::from_millis(750), worker)
            .await
            .context("checkout resolution timed out")??;
        resolved
    }
}

fn prepare_checkout(
    input: &mut ToolHookInput,
    phase: ToolHookPhase,
    explicit: Option<&Path>,
) -> Result<LoadedConfig> {
    let (directory, git) = execution(input)?;
    let mut root = repository_root(&directory).unwrap_or_else(|| directory.clone());
    resolve_file_paths(input, &directory, &mut root)?;
    input.context.project_root = Some(root.to_string_lossy().into_owned());
    if let Some(git) = git.filter(|_| phase == ToolHookPhase::Before) {
        input.client_path_intents.extend(git.incoming_paths(&root)?);
    }
    Config::load(explicit, Some(&root))
}

fn execution(input: &ToolHookInput) -> Result<(PathBuf, Option<LiteralGit>)> {
    let original = input
        .context
        .project_root
        .as_ref()
        .context("missing checkout")?;
    let original = std::fs::canonicalize(original).context("callback directory unavailable")?;
    let shell = shell_invocation(&input.tool_name, &input.tool_input);
    let git = shell
        .as_ref()
        .map(|shell| LiteralGit::parse(shell.command))
        .transpose()?
        .flatten();
    let directory = match shell {
        Some(shell) => shell_directory(shell.directory, &original)?,
        None => original,
    };
    let directory = match &git {
        Some(git) => git.execution_directory(&directory)?,
        None => directory,
    };
    Ok((directory, git))
}

fn shell_directory(reported: Option<&str>, original: &Path) -> Result<PathBuf> {
    let directory = match reported {
        Some(value) => {
            anyhow::ensure!(
                !value.is_empty() && value.len() <= 4096,
                "invalid execution directory"
            );
            let path = original
                .join(value)
                .canonicalize()
                .context("execution directory unavailable")?;
            anyhow::ensure!(path.is_dir(), "execution directory is not a directory");
            path
        }
        None => original.to_owned(),
    };
    Ok(directory)
}

fn resolve_file_paths(
    input: &mut crate::coordination::domain::ToolHookInput,
    directory: &Path,
    root: &mut PathBuf,
) -> Result<()> {
    let mut paths =
        crate::coordination::extract_path_intents(&input.tool_name, &input.tool_input, None);
    let mut roots = std::collections::BTreeSet::new();
    let mut directories = std::collections::BTreeSet::new();
    for intent in &mut paths {
        let path = directory.join(&intent.path);
        let parent = existing_parent(&path)?;
        if directories.insert(parent.to_path_buf()) {
            anyhow::ensure!(
                directories.len() <= 16,
                "tool checkout resolution directory limit exceeded"
            );
            roots.extend(repository_root(parent));
        }
        intent.path = path.to_string_lossy().into_owned();
    }
    anyhow::ensure!(
        roots.len() <= 1,
        "tool spans multiple repositories; use separate native calls for coordination"
    );
    if let Some(observed) = roots.into_iter().next() {
        *root = observed;
    }
    input.client_path_intents = paths;
    Ok(())
}

fn existing_parent(path: &Path) -> Result<&Path> {
    let mut parent = path.parent().context("file intent has no parent")?;
    while !parent.is_dir() {
        parent = parent
            .parent()
            .context("file intent has no accessible parent")?;
    }
    Ok(parent)
}

fn repository_root(directory: &Path) -> Option<PathBuf> {
    let mut command = std::process::Command::new("git");
    command
        .arg("-C")
        .arg(directory)
        .args(["rev-parse", "--show-toplevel"]);
    crate::git_process::bounded_output(command, Duration::from_millis(250), 8192)
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|root| PathBuf::from(root.trim()))
}
