// @feature usage-analytics
// @spec docs/features/usage-analytics.md
use super::types::*;
use anyhow::{bail, Result};
use std::path::Path;

pub fn validate(scope: &str, resources: &[Resource]) -> Result<()> {
    if !super::classification::valid_slug(scope) || resources.len() > MAX_RESOURCES {
        bail!("inventory scope or resource count exceeds bounds");
    }
    for resource in resources {
        if resource.name.is_empty()
            || resource.name.len() > 512
            || resource.name.chars().any(char::is_control)
            || resource.description.len() > 512
            || resource.description.chars().any(char::is_control)
            || resource.activities.len() > 8
            || resource
                .activities
                .iter()
                .any(|id| !super::classification::valid_slug(id))
            || resource
                .version
                .as_ref()
                .is_some_and(|v| v.len() > 128 || v.chars().any(char::is_control))
        {
            bail!("invalid inventory resource");
        }
        if resource.kind == ResourceKind::Script
            && (Path::new(&resource.name).is_absolute()
                || Path::new(&resource.name)
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir)))
        {
            bail!("script inventory paths must be repository relative");
        }
    }
    Ok(())
}

pub fn repository(root: &Path) -> RepositoryInventory {
    let mut snapshot = RepositoryInventory {
        resources: Vec::new(),
        complete: true,
    };
    let mut folders = std::collections::VecDeque::from([
        "scripts".to_owned(),
        "tools".to_owned(),
        ".githooks".to_owned(),
    ]);
    let started = std::time::Instant::now();
    let mut visited = 0;
    while let Some(folder) = folders.pop_front() {
        let entries = match std::fs::read_dir(root.join(&folder)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                snapshot.complete = false;
                continue;
            }
        };
        for entry in entries {
            visited += 1;
            if visited > 1024
                || snapshot.resources.len() >= MAX_RESOURCES
                || started.elapsed().as_millis() > 25
            {
                snapshot.complete = false;
                return snapshot;
            }
            match repository_entry(entry, &folder) {
                Some(RepositoryEntry::Folder(name)) => folders.push_back(name),
                Some(RepositoryEntry::File(name)) => snapshot.resources.push(Resource {
                    version: super::capture::script_version(root, &name),
                    name,
                    kind: ResourceKind::Script,
                    origin: Origin::Repository,
                    exposure: Exposure::Discoverable,
                    description: "Supplied repository script".into(),
                    activities: vec![],
                }),
                None => snapshot.complete = false,
            }
        }
    }
    snapshot.resources.sort_by(|a, b| a.name.cmp(&b.name));
    snapshot
}

pub fn capture_once(
    root: &Path,
    context: &crate::coordination::domain::HookContext,
) -> Option<RepositoryInventory> {
    use std::collections::VecDeque;
    use std::sync::{LazyLock, Mutex};
    static SEEN: LazyLock<Mutex<VecDeque<String>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));
    let key = super::identity(&[&root.to_string_lossy(), &context.agent, &context.session_id]);
    let mut seen = SEEN.try_lock().ok()?;
    if seen.contains(&key) {
        return None;
    }
    if seen.len() >= 512 {
        seen.pop_front();
    }
    seen.push_back(key);
    drop(seen);
    Some(repository(root))
}

enum RepositoryEntry {
    Folder(String),
    File(String),
}
fn repository_entry(
    entry: std::io::Result<std::fs::DirEntry>,
    folder: &str,
) -> Option<RepositoryEntry> {
    let entry = entry.ok()?;
    let metadata = entry.file_type().ok()?;
    let name = format!("{folder}/{}", entry.file_name().to_string_lossy());
    if name.len() > 512 || name.chars().any(char::is_control) {
        return None;
    }
    if metadata.is_dir() {
        Some(RepositoryEntry::Folder(name))
    } else if metadata.is_file() {
        Some(RepositoryEntry::File(name))
    } else {
        None
    }
}
