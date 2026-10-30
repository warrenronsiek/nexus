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

#[derive(Clone, Copy)]
pub(super) enum Window {
    Delivery,
    Eviction,
}
impl Window {
    fn includes(self, path: &Path) -> bool {
        match self {
            Self::Delivery => is_pending(path),
            Self::Eviction => !is_pending(path),
        }
    }
    fn priority(self, path: &Path) -> bool {
        match self {
            Self::Delivery => false,
            Self::Eviction => !path.extension().is_some_and(|ext| ext == "seen"),
        }
    }
}

pub(super) fn retain_smallest<T: Ord>(selected: &mut Vec<T>, item: T, limit: usize) {
    selected.push(item);
    selected.sort();
    selected.truncate(limit);
}

// Inventory every retained boundary, but keep only a bounded delivery or
// eviction window in memory. Legacy over-capacity directories must drain.
pub(super) fn oldest_entries(
    directory: &Path,
    window: Window,
    limit: usize,
) -> Result<Vec<PathBuf>> {
    let mut selected = Vec::new();
    for path in entries(directory)? {
        let path = path?;
        if !window.includes(&path) {
            continue;
        }
        let priority = (
            window.priority(&path),
            std::fs::metadata(&path).and_then(|m| m.modified()).ok(),
            path,
        );
        retain_smallest(&mut selected, priority, limit);
    }
    Ok(selected.into_iter().map(|(_, _, path)| path).collect())
}
