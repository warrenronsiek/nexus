// @feature work-history
// @spec docs/features/work-history.md
// @feature installation
// @feature sos
// @feature usage-analytics
// @feature evals
// @spec docs/features/installation.md
// @spec docs/features/sos.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/evals.md
// @entrypoint setup_machine
// @entrypoint install_repository
// @boundary dynamic-json
use crate::agents::integration::{hook_configuration, mcp_server_arguments, Host};
use crate::config::{ClientConfig, Config};
use crate::persistence::Store;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PROJECT_CONFIG: &str = "schema_version = 1\n";
const SKILL_NAMES: [&str; 17] = [
    "code-architect",
    "code-deletion",
    "tdd",
    "nexus",
    "nexus-setup",
    "nexus-coordination",
    "nexus-agent-memory",
    "nexus-papercuts",
    "nexus-sos",
    "nexus-usage-analytics",
    "nexus-observability",
    "nexus-development",
    "nexus-build-eval",
    "nexus-eval-audit",
    "nexus-hillclimb",
    "nexus-eval-history",
    "nexus-work-history",
];
const SKILL_FILES: [(&str, &str); 26] = [
    (
        "code-architect/SKILL.md",
        include_str!("../skills/code-architect/SKILL.md"),
    ),
    (
        "code-architect/references/feature-mapping.md",
        include_str!("../skills/code-architect/references/feature-mapping.md"),
    ),
    (
        "code-deletion/SKILL.md",
        include_str!("../skills/code-deletion/SKILL.md"),
    ),
    ("nexus/SKILL.md", include_str!("../skills/nexus/SKILL.md")),
    (
        "nexus-agent-memory/SKILL.md",
        include_str!("../skills/nexus-agent-memory/SKILL.md"),
    ),
    (
        "nexus-build-eval/SKILL.md",
        include_str!("../skills/nexus-build-eval/SKILL.md"),
    ),
    (
        "nexus-coordination/SKILL.md",
        include_str!("../skills/nexus-coordination/SKILL.md"),
    ),
    (
        "nexus-development/SKILL.md",
        include_str!("../skills/nexus-development/SKILL.md"),
    ),
    (
        "nexus-development/references/persistence-validation.md",
        include_str!("../skills/nexus-development/references/persistence-validation.md"),
    ),
    (
        "nexus-development/references/source-structure.md",
        include_str!("../skills/nexus-development/references/source-structure.md"),
    ),
    (
        "nexus-eval-audit/SKILL.md",
        include_str!("../skills/nexus-eval-audit/SKILL.md"),
    ),
    (
        "nexus-eval-history/SKILL.md",
        include_str!("../skills/nexus-eval-history/SKILL.md"),
    ),
    (
        "nexus-eval-history/references/nexus-evals.md",
        include_str!("../skills/nexus-eval-history/references/nexus-evals.md"),
    ),
    (
        "nexus-hillclimb/SKILL.md",
        include_str!("../skills/nexus-hillclimb/SKILL.md"),
    ),
    (
        "nexus-observability/SKILL.md",
        include_str!("../skills/nexus-observability/SKILL.md"),
    ),
    (
        "nexus-papercuts/SKILL.md",
        include_str!("../skills/nexus-papercuts/SKILL.md"),
    ),
    (
        "nexus-setup/SKILL.md",
        include_str!("../skills/nexus-setup/SKILL.md"),
    ),
    (
        "nexus-setup/references/integration.md",
        include_str!("../skills/nexus-setup/references/integration.md"),
    ),
    (
        "nexus-sos/SKILL.md",
        include_str!("../skills/nexus-sos/SKILL.md"),
    ),
    (
        "nexus-usage-analytics/SKILL.md",
        include_str!("../skills/nexus-usage-analytics/SKILL.md"),
    ),
    (
        "nexus-usage-analytics/references/integration.md",
        include_str!("../skills/nexus-usage-analytics/references/integration.md"),
    ),
    (
        "nexus-work-history/SKILL.md",
        include_str!("../skills/nexus-work-history/SKILL.md"),
    ),
    (
        "nexus-work-history/references/integration.md",
        include_str!("../skills/nexus-work-history/references/integration.md"),
    ),
    ("tdd/SKILL.md", include_str!("../skills/tdd/SKILL.md")),
    (
        "nexus-work-history/scripts/work-capture.sh",
        include_str!("../scripts/work-capture.sh"),
    ),
    (
        "nexus-work-history/scripts/nexus-work-github.py",
        include_str!("../scripts/nexus-work-github.py"),
    ),
];
const PI_EXTENSION_FILES: [(&str, &str); 40] = [
    ("index.ts", include_str!("../pi-extension/src/index.ts")),
    (
        "history-pane.ts",
        include_str!("../pi-extension/src/history-pane.ts"),
    ),
    (
        "text-filter.ts",
        include_str!("../pi-extension/src/text-filter.ts"),
    ),
    (
        "wire-values.ts",
        include_str!("../pi-extension/src/wire-values.ts"),
    ),
    (
        "work-navigation.ts",
        include_str!("../pi-extension/src/work-navigation.ts"),
    ),
    (
        "work-requests.ts",
        include_str!("../pi-extension/src/work-requests.ts"),
    ),
    (
        "analytics-data.ts",
        include_str!("../pi-extension/src/analytics-data.ts"),
    ),
    (
        "analytics-requests.ts",
        include_str!("../pi-extension/src/analytics-requests.ts"),
    ),
    (
        "analytics-view.ts",
        include_str!("../pi-extension/src/analytics-view.ts"),
    ),
    (
        "analytics.ts",
        include_str!("../pi-extension/src/analytics.ts"),
    ),
    (
        "inventory-capture.ts",
        include_str!("../pi-extension/src/inventory-capture.ts"),
    ),
    (
        "papercuts-pane.ts",
        include_str!("../pi-extension/src/papercuts-pane.ts"),
    ),
    (
        "papercuts.ts",
        include_str!("../pi-extension/src/papercuts.ts"),
    ),
    (
        "work-pane.ts",
        include_str!("../pi-extension/src/work-pane.ts"),
    ),
    (
        "work-state.ts",
        include_str!("../pi-extension/src/work-state.ts"),
    ),
    (
        "work-view.ts",
        include_str!("../pi-extension/src/work-view.ts"),
    ),
    ("work.ts", include_str!("../pi-extension/src/work.ts")),
    ("capture.ts", include_str!("../pi-extension/src/capture.ts")),
    ("client.ts", include_str!("../pi-extension/src/client.ts")),
    (
        "component.ts",
        include_str!("../pi-extension/src/component.ts"),
    ),
    ("display.ts", include_str!("../pi-extension/src/display.ts")),
    ("domain.ts", include_str!("../pi-extension/src/domain.ts")),
    ("evals.ts", include_str!("../pi-extension/src/evals.ts")),
    (
        "evals-pane.ts",
        include_str!("../pi-extension/src/evals-pane.ts"),
    ),
    (
        "evals-view.ts",
        include_str!("../pi-extension/src/evals-view.ts"),
    ),
    (
        "evals-state.ts",
        include_str!("../pi-extension/src/evals-state.ts"),
    ),
    (
        "evals-requests.ts",
        include_str!("../pi-extension/src/evals-requests.ts"),
    ),
    (
        "host-context.ts",
        include_str!("../pi-extension/src/host-context.ts"),
    ),
    (
        "mcp-client.ts",
        include_str!("../pi-extension/src/mcp-client.ts"),
    ),
    (
        "memory-context.ts",
        include_str!("../pi-extension/src/memory-context.ts"),
    ),
    (
        "memory-pane.ts",
        include_str!("../pi-extension/src/memory-pane.ts"),
    ),
    (
        "memory-prompts.ts",
        include_str!("../pi-extension/src/memory-prompts.ts"),
    ),
    ("memory.ts", include_str!("../pi-extension/src/memory.ts")),
    (
        "navigation.ts",
        include_str!("../pi-extension/src/navigation.ts"),
    ),
    ("polling.ts", include_str!("../pi-extension/src/polling.ts")),
    (
        "sos-pane.ts",
        include_str!("../pi-extension/src/sos-pane.ts"),
    ),
    ("sos.ts", include_str!("../pi-extension/src/sos.ts")),
    ("theme.ts", include_str!("../pi-extension/src/theme.ts")),
    (
        "usage-pane.ts",
        include_str!("../pi-extension/src/usage-pane.ts"),
    ),
    ("usage.ts", include_str!("../pi-extension/src/usage.ts")),
];

#[derive(Debug, Serialize)]
pub struct MachineSetupReport {
    pub status: &'static str,
    pub executable: PathBuf,
    pub hosts: Vec<String>,
    pub skills: Vec<&'static str>,
    pub extensions: Vec<&'static str>,
    pub database: &'static str,
    pub mode: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RepositoryInstallReport {
    pub status: &'static str,
    pub repository: PathBuf,
    pub config: PathBuf,
    pub database: &'static str,
    pub mode: &'static str,
}

pub fn setup_machine(
    executable: &Path,
    explicit_config: Option<&Path>,
) -> Result<MachineSetupReport> {
    let home = dirs::home_dir().context("resolve home directory")?;
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let claude_home = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let (hosts, extensions) = install_agent_resources(
        executable,
        explicit_config,
        &home,
        &codex_home,
        &claude_home,
    )?;
    if hosts.is_empty() {
        bail!(
            "neither codex nor claude is installed; install an agent host and rerun `nexus setup`"
        );
    }

    let loaded = Config::load(explicit_config, None)?;
    let database = match &loaded.config.client {
        ClientConfig::Local => {
            let _store = Store::open(&loaded.config.storage.database_path)?;
            "ready"
        }
        ClientConfig::Remote(_) => "remote",
    };
    Ok(MachineSetupReport {
        status: "ready",
        executable: executable.to_path_buf(),
        hosts,
        skills: SKILL_NAMES.to_vec(),
        extensions,
        database,
        mode: "advisory_only",
    })
}

fn install_agent_resources(
    executable: &Path,
    explicit_config: Option<&Path>,
    home: &Path,
    codex_home: &Path,
    claude_home: &Path,
) -> Result<(Vec<String>, Vec<&'static str>)> {
    for root in [codex_home, claude_home] {
        install_embedded_files(&root.join("skills"), &SKILL_FILES)?;
    }
    let hosts = install_host_integrations(executable, explicit_config, codex_home, claude_home)?;
    let extensions = if command_available("pi") {
        install_pi_extension(
            &home.join(".pi/agent/extensions/nexus"),
            executable,
            explicit_config,
        )?;
        vec!["pi:nexus"]
    } else {
        Vec::new()
    };
    Ok((hosts, extensions))
}

fn install_pi_extension(
    root: &Path,
    executable: &Path,
    explicit_config: Option<&Path>,
) -> Result<()> {
    install_embedded_files(root, &PI_EXTENSION_FILES)?;
    let launch_context = pi_launch_context(executable, explicit_config)?;
    install_embedded_files(root, &[("launch-context.ts", launch_context.as_str())])
}

fn pi_launch_context(executable: &Path, explicit_config: Option<&Path>) -> Result<String> {
    let executable = executable
        .to_str()
        .context("Nexus executable path must be UTF-8 for the Pi extension")?;
    let executable = serde_json::to_string(executable)?;
    let config = explicit_config
        .map(|path| {
            path.to_str()
                .context("Nexus configuration path must be UTF-8 for the Pi extension")
                .and_then(|path| serde_json::to_string(path).map_err(Into::into))
        })
        .transpose()?
        .map(|config| format!("  config: {config},\n"))
        .unwrap_or_default();
    Ok(format!(
        "// @feature runtime\n\
// @feature installation\n\
// @spec docs/features/runtime.md\n\
// @spec docs/features/installation.md\n\
\n\
export interface NexusLaunchContext {{\n\
  executable: string;\n\
  config?: string;\n\
}}\n\
\n\
export const nexusLaunchContext: NexusLaunchContext = {{\n\
  executable: {executable},\n\
{config}}};\n"
    ))
}

fn install_host_integrations(
    executable: &Path,
    explicit_config: Option<&Path>,
    codex_home: &Path,
    claude_home: &Path,
) -> Result<Vec<String>> {
    let mut hosts = Vec::new();
    for (host, settings) in [
        (Host::Codex, codex_home.join("hooks.json")),
        (Host::Claude, claude_home.join("settings.json")),
    ] {
        if command_available(host.name()) {
            register_mcp(host, executable, explicit_config)?;
            merge_hooks(
                &settings,
                hook_configuration(host, executable, explicit_config),
            )?;
            hosts.push(host.name().to_owned());
        }
    }
    Ok(hosts)
}

pub fn install_repository(
    repository: &Path,
    explicit_config: Option<&Path>,
) -> Result<RepositoryInstallReport> {
    let repository = repository
        .canonicalize()
        .with_context(|| format!("resolve repository {}", repository.display()))?;
    let common_directory = repository_common_directory(&repository)?;
    let project_config = common_directory.join("nexus.toml");
    if !project_config.exists() {
        std::fs::write(&project_config, PROJECT_CONFIG)
            .with_context(|| format!("write {}", project_config.display()))?;
    }
    let loaded = Config::load(explicit_config, Some(&repository))?;
    let database = match &loaded.config.client {
        ClientConfig::Local => {
            let _store = Store::open(&loaded.config.storage.database_path)?;
            "ready"
        }
        ClientConfig::Remote(_) => "remote",
    };
    Ok(RepositoryInstallReport {
        status: "ready",
        repository,
        config: project_config,
        database,
        mode: "advisory_only",
    })
}

fn repository_common_directory(repository: &Path) -> Result<PathBuf> {
    crate::git_process::common_directory(repository)
        .context("read Git common directory")?
        .with_context(|| format!("{} is not a Git repository", repository.display()))
}

fn install_embedded_files(root: &Path, files: &[(&str, &str)]) -> Result<()> {
    for (relative_path, contents) in files {
        let path = root.join(relative_path);
        let directory = path.parent().context("embedded file must have a parent")?;
        std::fs::create_dir_all(directory)
            .with_context(|| format!("create {}", directory.display()))?;
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*contents) {
            std::fs::write(&path, contents).with_context(|| format!("write {}", path.display()))?;
        }
    }
    Ok(())
}

fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn register_mcp(host: Host, executable: &Path, explicit_config: Option<&Path>) -> Result<()> {
    let remove: Vec<OsString> = match host {
        Host::Codex => ["mcp", "remove", "nexus"]
            .into_iter()
            .map(OsString::from)
            .collect(),
        Host::Claude => ["mcp", "remove", "--scope", "user", "nexus"]
            .into_iter()
            .map(OsString::from)
            .collect(),
    };
    let mut add: Vec<OsString> = match host {
        Host::Codex => ["mcp", "add", "nexus", "--"]
            .into_iter()
            .map(OsString::from)
            .collect(),
        Host::Claude => [
            "mcp",
            "add",
            "--transport",
            "stdio",
            "--scope",
            "user",
            "nexus",
            "--",
        ]
        .into_iter()
        .map(OsString::from)
        .collect(),
    };
    add.extend(mcp_server_arguments(host, executable, explicit_config));
    let _ = Command::new(host.name())
        .args(remove)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let output = Command::new(host.name())
        .args(add)
        .output()
        .with_context(|| format!("register Nexus with {}", host.name()))?;
    if !output.status.success() {
        bail!(
            "{} MCP registration failed: {}",
            host.name(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn merge_hooks(path: &Path, generated: Value) -> Result<()> {
    let mut settings = if path.exists() {
        serde_json::from_slice::<Value>(&std::fs::read(path)?)
            .with_context(|| format!("parse {}", path.display()))?
    } else {
        Value::Object(Default::default())
    };
    let settings_object = settings
        .as_object_mut()
        .context("agent settings root must be a JSON object")?;
    let hooks = settings_object
        .entry("hooks")
        .or_insert_with(|| Value::Object(Default::default()))
        .as_object_mut()
        .context("agent hooks setting must be a JSON object")?;
    let generated_hooks = generated
        .get("hooks")
        .and_then(Value::as_object)
        .context("generated hooks must be a JSON object")?;
    for (event, generated_entries) in generated_hooks {
        let entries = hooks
            .entry(event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .with_context(|| format!("hook event {event} must be an array"))?;
        entries.retain(|entry| !contains_nexus_hook(entry));
        entries.extend(
            generated_entries
                .as_array()
                .with_context(|| format!("generated hook event {event} must be an array"))?
                .iter()
                .cloned(),
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(&settings)?)
        .with_context(|| format!("write {}", path.display()))
}

fn contains_nexus_hook(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.get("server").and_then(Value::as_str) == Some("nexus")
                || object
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| command.contains(" hook-session-end --agent "))
                || object.values().any(contains_nexus_hook)
        }
        Value::Array(values) => values.iter().any(contains_nexus_hook),
        _ => false,
    }
}
