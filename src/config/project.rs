// @feature configuration
// @spec docs/features/configuration.md
// @boundary git-process-output
use std::path::{Path, PathBuf};

pub(super) fn project_config_path(root: &Path) -> Option<PathBuf> {
    crate::git_process::common_directory(root)
        .ok()
        .flatten()
        .map(|common| common.join("nexus.toml"))
}

const MAX_PROJECT_NAME_BYTES: usize = 128;

/// Display names label a project ID; invalid names are omitted rather than rejected.
pub(crate) fn canonical_project_name(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= MAX_PROJECT_NAME_BYTES
        && !value.chars().any(char::is_control))
    .then(|| value.to_owned())
}

pub(super) fn drop_local_project_name(value: &mut toml::Value) {
    if let Some(client) = value.get_mut("client").and_then(toml::Value::as_table_mut) {
        if client.get("transport").and_then(toml::Value::as_str) != Some("remote") {
            client.remove("project_name");
        }
    }
}

/// A repository binding owns both its project ID and label; an inherited user-level
/// label would otherwise name every bound repository after the fallback project.
pub(super) fn name_repository_binding(value: &mut toml::Value, overlay: &toml::Value, path: &Path) {
    let bound = overlay
        .get("client")
        .and_then(|client| client.get("project_id"));
    let named = overlay
        .get("client")
        .and_then(|client| client.get("project_name"));
    if bound.is_none() || named.is_some() {
        return;
    }
    let Some(client) = value.get_mut("client").and_then(toml::Value::as_table_mut) else {
        return;
    };
    match path.parent().and_then(crate::git_process::repository_name) {
        Some(name) => client.insert("project_name".into(), toml::Value::String(name)),
        None => client.remove("project_name"),
    };
}
