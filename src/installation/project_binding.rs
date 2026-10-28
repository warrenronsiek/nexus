// @boundary project-toml: preserve caller-owned settings while updating only a project binding.
// @feature installation
// @feature configuration
// @feature coordination
// @spec docs/features/installation.md
// @spec docs/features/configuration.md
// @spec docs/features/coordination.md
use crate::config::{ClientConfig, Config};
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub(super) fn share(
    target: &Path,
    source: &Path,
    configuration: &Path,
    explicit: Option<&Path>,
) -> Result<()> {
    let source = source
        .canonicalize()
        .context("resolve source project checkout")?;
    super::repository_common_directory(&source)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(configuration.with_extension("toml.lock"))?;
    FileExt::try_lock_exclusive(&lock).context("project binding is being updated")?;
    let project_id = shared_project(target, &source, explicit)?;
    write_binding(configuration, project_id)
}

fn shared_project(target: &Path, source: &Path, explicit: Option<&Path>) -> Result<String> {
    let source = Config::load(explicit, Some(source))?;
    let target = Config::load(explicit, Some(target))?;
    let (ClientConfig::Remote(source), ClientConfig::Remote(target)) =
        (&source.config.client, &target.config.client)
    else {
        bail!("sharing independent clones requires the same remote Nexus service");
    };
    if source.endpoint.trim_end_matches('/') != target.endpoint.trim_end_matches('/') {
        bail!("cannot share project bindings across different Nexus services");
    }
    Ok(source.project_id.clone())
}

fn write_binding(configuration: &Path, project_id: String) -> Result<()> {
    let mut value: toml::Value = toml::from_str(&std::fs::read_to_string(configuration)?)?;
    let object = value
        .as_table_mut()
        .context("project configuration must be a table")?;
    let client = object
        .entry("client")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .context("client configuration must be a table")?;
    client.insert("project_id".into(), toml::Value::String(project_id));
    publish(configuration, &toml::to_string_pretty(&value)?)
}

fn publish(configuration: &Path, contents: &str) -> Result<()> {
    let mut staged = tempfile::NamedTempFile::new_in(
        configuration
            .parent()
            .context("project directory missing")?,
    )?;
    staged.write_all(contents.as_bytes())?;
    staged.as_file().sync_all()?;
    staged
        .persist(configuration)
        .context("publish shared project binding")?;
    Ok(())
}
