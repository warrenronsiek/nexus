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
