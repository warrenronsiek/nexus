// @feature coordination
// @spec docs/features/coordination.md
use super::domain::Operation;
use anyhow::{Context, Result};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::Duration;

const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const GIT_STREAM_CAPTURE_LIMIT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct ProjectIdentity {
    pub(super) id: String,
    pub(super) claim_root: Option<String>,
    pub(super) worktree: Option<String>,
    pub(super) display_name: Option<String>,
}

/// Who is asking on behalf of a project. Agent bindings record project activity;
/// browser inspection reads a project without making it look recently active.
#[derive(Debug, Clone)]
pub(crate) enum ProjectBinding {
    Local,
    Remote {
        project_id: String,
        display_name: Option<String>,
    },
    Inspect {
        project_id: Option<String>,
    },
}

impl ProjectBinding {
    pub(crate) fn remote(project_id: impl Into<String>, display_name: Option<String>) -> Self {
        Self::Remote {
            project_id: project_id.into(),
            display_name: display_name
                .as_deref()
                .and_then(crate::config::canonical_project_name),
        }
    }

    fn selected_project(&self) -> Option<&str> {
        match self {
            Self::Local | Self::Inspect { project_id: None } => None,
            Self::Remote { project_id, .. }
            | Self::Inspect {
                project_id: Some(project_id),
            } => Some(project_id.trim()),
        }
    }

    /// A selected project ID comes from the caller rather than this host's filesystem.
    pub(crate) fn is_remote(&self) -> bool {
        self.selected_project().is_some()
    }

    pub(crate) fn records_activity(&self) -> bool {
        !matches!(self, Self::Inspect { .. })
    }

    pub(super) fn default_read_project(&self, requested: Option<String>) -> Option<String> {
        requested.or_else(|| self.selected_project().map(str::to_owned))
    }
}

pub(super) fn identify(
    binding: &ProjectBinding,
    project_root: Option<&str>,
) -> Result<ProjectIdentity> {
    if let Some(project_id) = binding.selected_project() {
        return selected_identity(binding, project_id, project_root);
    }
    let root = project_root
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let canonical_root = std::fs::canonicalize(&root).unwrap_or(root);
    let output = git_output([
        "-C",
        canonical_root
            .to_str()
            .context("project root is not UTF-8")?,
        "rev-parse",
        "--path-format=absolute",
        "--git-common-dir",
    ]);
    let (identity_path, display_name) = match output {
        Ok(output) if output.status.success() => {
            let value = String::from_utf8(output.stdout)?.trim().to_owned();
            let common = std::fs::canonicalize(&value).unwrap_or_else(|_| PathBuf::from(value));
            let name = crate::git_process::repository_name(&common);
            (common, name)
        }
        _ => (canonical_root.clone(), directory_name(&canonical_root)),
    };
    let id = blake3::hash(identity_path.to_string_lossy().as_bytes()).to_hex()[..24].to_owned();
    Ok(ProjectIdentity {
        id,
        claim_root: Some(canonical_root.to_string_lossy().into_owned()),
        worktree: Some(canonical_root.to_string_lossy().into_owned()),
        display_name,
    })
}

fn selected_identity(
    binding: &ProjectBinding,
    project_id: &str,
    project_root: Option<&str>,
) -> Result<ProjectIdentity> {
    if project_id.is_empty() {
        anyhow::bail!("remote project id cannot be empty");
    }
    let display_name = match binding {
        ProjectBinding::Remote { display_name, .. } => display_name.clone(),
        _ => None,
    };
    Ok(ProjectIdentity {
        id: project_id.to_owned(),
        claim_root: project_root.map(str::to_owned),
        worktree: None,
        display_name,
    })
}

fn directory_name(root: &std::path::Path) -> Option<String> {
    root.file_name()
        .and_then(|name| name.to_str())
        .and_then(crate::config::canonical_project_name)
}

pub(crate) struct ChangedPathObservation {
    output: Vec<u8>,
}

pub(crate) fn observe_changed_paths(worktree: &str) -> Option<ChangedPathObservation> {
    let output = git_output([
        "-C",
        worktree,
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
    ]);
    let Ok(output) = output else {
        return None;
    };
    if !output.status.success() {
        return None;
    }
    Some(ChangedPathObservation {
        output: output.stdout,
    })
}

impl ChangedPathObservation {
    pub(crate) fn visit(&self, mut visitor: impl FnMut(&str, Operation) -> ControlFlow<()>) {
        let mut fields = self
            .output
            .split(|byte| *byte == 0)
            .filter(|field| !field.is_empty());
        while let Some(field) = fields.next() {
            if field.len() < 4 {
                continue;
            }
            let status = &field[..2];
            let path = String::from_utf8_lossy(&field[3..]);
            let operation = if status.contains(&b'D') {
                Operation::Delete
            } else if status.contains(&b'R') {
                Operation::Rename
            } else {
                Operation::Write
            };
            if visitor(&path, operation).is_break() {
                return;
            }
            if status.contains(&b'R') || status.contains(&b'C') {
                let _ = fields.next();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn from_output(output: Vec<u8>) -> Self {
        Self { output }
    }
}

fn git_output<const N: usize>(arguments: [&str; N]) -> std::io::Result<Output> {
    let mut command = Command::new("git");
    command.args(arguments);
    command_output_with_timeout(command, GIT_COMMAND_TIMEOUT)
}

fn command_output_with_timeout(command: Command, timeout: Duration) -> std::io::Result<Output> {
    command_output_with_capture_limit(command, timeout, GIT_STREAM_CAPTURE_LIMIT_BYTES)
}

fn command_output_with_capture_limit(
    command: Command,
    timeout: Duration,
    stream_limit: usize,
) -> std::io::Result<Output> {
    crate::git_process::bounded_output(command, timeout, stream_limit)
}

#[cfg(test)]
mod tests;
