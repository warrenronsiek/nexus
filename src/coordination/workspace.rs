// @feature coordination
// @spec docs/features/coordination.md
use super::domain::{Operation, PathIntent};
use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Debug)]
pub(super) struct ProjectIdentity {
    pub(super) id: String,
    pub(super) worktree: Option<String>,
}

pub(super) fn identify(project_root: Option<&str>) -> Result<ProjectIdentity> {
    let root = project_root
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let canonical_root = std::fs::canonicalize(&root).unwrap_or(root);
    let output = std::process::Command::new("git")
        .args([
            "-C",
            canonical_root
                .to_str()
                .context("project root is not UTF-8")?,
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
        ])
        .output();
    let identity_path = match output {
        Ok(output) if output.status.success() => {
            let value = String::from_utf8(output.stdout)?.trim().to_owned();
            std::fs::canonicalize(&value).unwrap_or_else(|_| PathBuf::from(value))
        }
        _ => canonical_root.clone(),
    };
    let id = blake3::hash(identity_path.to_string_lossy().as_bytes()).to_hex()[..24].to_owned();
    Ok(ProjectIdentity {
        id,
        worktree: Some(canonical_root.to_string_lossy().into_owned()),
    })
}

pub(super) fn changed_paths(worktree: &str) -> Vec<PathIntent> {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            worktree,
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let mut fields = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut intents = Vec::new();
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        let status = &field[..2];
        let path = String::from_utf8_lossy(&field[3..]).into_owned();
        let operation = if status.contains(&b'D') {
            Operation::Delete
        } else if status.contains(&b'R') {
            Operation::Rename
        } else {
            Operation::Write
        };
        intents.push(PathIntent {
            path,
            operation,
            line_start: None,
            line_end: None,
        });
        if status.contains(&b'R') || status.contains(&b'C') {
            let _ = fields.next();
        }
    }
    intents
}
