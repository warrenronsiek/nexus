// @feature installation
// @spec docs/features/installation.md
// @entrypoint setup_machine
// @entrypoint install_repository
// @boundary dynamic-json
use crate::agents::integration::{hook_configuration, Host};
use crate::config::Config;
use crate::persistence::Store;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PROJECT_CONFIG: &str = "schema_version = 1\n";
const SKILL_NAMES: [&str; 3] = ["code-architect", "code-deletion", "tdd"];
const SKILL_FILES: [(&str, &str); 4] = [
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
    ("tdd/SKILL.md", include_str!("../skills/tdd/SKILL.md")),
];

#[derive(Debug, Serialize)]
pub struct MachineSetupReport {
    pub status: &'static str,
    pub executable: PathBuf,
    pub hosts: Vec<String>,
    pub skills: Vec<&'static str>,
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

pub fn setup_machine(executable: &Path) -> Result<MachineSetupReport> {
    let home = dirs::home_dir().context("resolve home directory")?;
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let claude_home = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));

    for root in [&codex_home, &claude_home] {
        install_skills(root)?;
    }

    let mut hosts = Vec::new();
    for (host, settings) in [
        (Host::Codex, codex_home.join("hooks.json")),
        (Host::Claude, claude_home.join("settings.json")),
    ] {
        if host_available(host) {
            register_mcp(host, executable)?;
            merge_hooks(&settings, hook_configuration(host))?;
            hosts.push(host.name().to_owned());
        }
    }
    if hosts.is_empty() {
        bail!(
            "neither codex nor claude is installed; install an agent host and rerun `nexus setup`"
        );
    }

    let loaded = Config::load(None, None)?;
    let _store = Store::open(&loaded.config.storage.database_path)?;
    Ok(MachineSetupReport {
        status: "ready",
        executable: executable.to_path_buf(),
        hosts,
        skills: SKILL_NAMES.to_vec(),
        database: "ready",
        mode: "advisory_only",
    })
}

pub fn install_repository(
    repository: &Path,
    explicit_config: Option<&Path>,
) -> Result<RepositoryInstallReport> {
    let repository = repository
        .canonicalize()
        .with_context(|| format!("resolve repository {}", repository.display()))?;
    let common_directory = git_common_directory(&repository)?;
    let project_config = common_directory.join("nexus.toml");
    if !project_config.exists() {
        std::fs::write(&project_config, PROJECT_CONFIG)
            .with_context(|| format!("write {}", project_config.display()))?;
    }
    let loaded = Config::load(explicit_config, Some(&repository))?;
    let _store = Store::open(&loaded.config.storage.database_path)?;
    Ok(RepositoryInstallReport {
        status: "ready",
        repository,
        config: project_config,
        database: "ready",
        mode: "advisory_only",
    })
}

fn install_skills(root: &Path) -> Result<()> {
    for (relative_path, contents) in SKILL_FILES {
        let path = root.join("skills").join(relative_path);
        let directory = path.parent().context("skill file must have a parent")?;
        std::fs::create_dir_all(directory)
            .with_context(|| format!("create {}", directory.display()))?;
        if std::fs::read_to_string(&path).ok().as_deref() != Some(contents) {
            std::fs::write(&path, contents).with_context(|| format!("write {}", path.display()))?;
        }
    }
    Ok(())
}

fn host_available(host: Host) -> bool {
    Command::new(host.name())
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn register_mcp(host: Host, executable: &Path) -> Result<()> {
    let executable = executable.as_os_str();
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
    add.extend([executable.to_owned(), OsString::from("mcp")]);
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
        entries.retain(|entry| !contains_nexus_server(entry));
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

fn contains_nexus_server(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.get("server").and_then(Value::as_str) == Some("nexus")
                || object.values().any(contains_nexus_server)
        }
        Value::Array(values) => values.iter().any(contains_nexus_server),
        _ => false,
    }
}

fn git_common_directory(repository: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(repository)
        .output()
        .context("read Git common directory")?;
    if !output.status.success() {
        bail!("{} is not a Git repository", repository.display());
    }
    let raw = String::from_utf8(output.stdout).context("Git common directory was not UTF-8")?;
    let path = PathBuf::from(raw.trim());
    Ok(if path.is_absolute() {
        path
    } else {
        repository.join(path)
    })
}
