// @feature work-history
// @spec docs/features/work-history.md
use super::is_pending;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub(super) fn entries(directory: &Path) -> Result<impl Iterator<Item = Result<PathBuf>>> {
    Ok(std::fs::read_dir(directory)?.filter_map(|entry| {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return Some(Err(error.into())),
        };
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => return None,
            Err(error) => return Some(Err(error.into())),
            _ => {}
        }
        let path = entry.path();
        path.file_name()
            .is_some_and(|name| !name.to_string_lossy().starts_with('.'))
            .then_some(Ok(path))
    }))
}

// Inventory every retained boundary, but keep only a bounded delivery or
// eviction window in memory. Legacy over-capacity directories must drain.
pub(super) fn oldest_entries(
    directory: &Path,
    pending: bool,
    limit: usize,
) -> Result<Vec<PathBuf>> {
    let mut selected = Vec::new();
    for path in entries(directory)? {
        let path = path?;
        if is_pending(&path) != pending {
            continue;
        }
        let priority = (
            !pending && !path.extension().is_some_and(|ext| ext == "seen"),
            std::fs::metadata(&path).and_then(|m| m.modified()).ok(),
            path,
        );
        selected.push(priority);
        selected.sort();
        selected.truncate(limit);
    }
    Ok(selected.into_iter().map(|(_, _, path)| path).collect())
}
