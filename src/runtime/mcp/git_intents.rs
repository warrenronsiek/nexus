// @feature coordination
// @feature runtime
// @spec docs/features/coordination.md
// @spec docs/features/runtime.md
// @boundary client-git-preflight: read bounded committed path metadata without executing the requested integration command or consulting external diff drivers.
use crate::coordination::domain::{Operation, PathIntent};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(super) struct LiteralGit {
    words: Vec<String>,
    index: usize,
    orchestrated: bool,
}

impl LiteralGit {
    pub(super) fn parse(command: &str) -> Result<Option<Self>> {
        anyhow::ensure!(
            command.len() <= 1024 * 1024,
            "Git preflight command exceeds capture limit"
        );
        Ok(literal_git(command).map(|(words, index)| Self {
            orchestrated: command.contains('\n')
                || words
                    .iter()
                    .any(|word| matches!(word.as_str(), "&&" | "||" | ";" | "|")),
            words,
            index,
        }))
    }

    pub(super) fn incoming_paths(&self, root: &Path) -> Result<Vec<PathIntent>> {
        let Some(operation) = self
            .words
            .get(self.index)
            .map(String::as_str)
            .filter(|op| matches!(*op, "merge" | "cherry-pick"))
        else {
            return Ok(Vec::new());
        };
        anyhow::ensure!(!self.orchestrated, "Git preflight requires a literal integration command; split shell orchestration into native calls");
        let mut intents = Vec::new();
        for target in integration_targets(&self.words[self.index + 1..])? {
            append_revision(&mut intents, root, operation, target)?;
        }
        Ok(intents)
    }

    pub(super) fn execution_directory(&self, base: &Path) -> Result<PathBuf> {
        let mut directory = base.to_owned();
        for pair in self.words[1..self.index].chunks_exact(2) {
            directory = directory.join(&pair[1]);
        }
        directory
            .canonicalize()
            .context("Git execution directory unavailable")
    }
}

fn append_revision(
    intents: &mut Vec<PathIntent>,
    root: &Path,
    operation: &str,
    target: &str,
) -> Result<()> {
    let revision = resolve_revision(root, target)?;
    append_metadata(intents, &changed_paths(root, operation, &revision)?)
}

fn integration_targets(words: &[String]) -> Result<Vec<&str>> {
    let mut targets = Vec::new();
    for word in words.iter() {
        if matches!(
            word.as_str(),
            "--no-commit"
                | "--no-ff"
                | "--ff-only"
                | "--ff"
                | "--squash"
                | "--no-edit"
                | "--allow-empty"
                | "-n"
                | "-x"
                | "--"
        ) {
            continue;
        }
        if word.starts_with('-') {
            bail!("Git preflight cannot account for this integration option");
        }
        targets.push(word.as_str());
    }
    if targets.len() > 8 {
        bail!("Git preflight target limit exceeded");
    }
    Ok(targets)
}

fn resolve_revision(root: &Path, target: &str) -> Result<String> {
    let revision = git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{target}^{{commit}}"),
        ],
    )?;
    let revision = std::str::from_utf8(&revision)?.trim();
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("invalid Git preflight revision");
    }
    Ok(revision.to_owned())
}

fn changed_paths(root: &Path, operation: &str, revision: &str) -> Result<Vec<u8>> {
    let output = if operation == "merge" {
        git(
            root,
            &[
                "diff",
                "--name-status",
                "-z",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                &format!("HEAD...{revision}"),
                "--",
            ],
        )?
    } else {
        git(
            root,
            &[
                "show",
                "--format=",
                "--name-status",
                "-z",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                revision,
                "--",
            ],
        )?
    };
    Ok(output)
}

fn append_metadata(intents: &mut Vec<PathIntent>, output: &[u8]) -> Result<()> {
    let mut fields = output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    while let Some(status) = fields.next() {
        let path = fields
            .next()
            .context("incomplete Git preflight path metadata")?;
        if intents.len() >= 256 {
            bail!("Git preflight path limit exceeded");
        }
        let path = std::str::from_utf8(path)?;
        if path.len() > 4096 {
            bail!("Git preflight path exceeds capture limit");
        }
        intents.push(PathIntent {
            path: path.to_owned(),
            operation: if status.first() == Some(&b'D') {
                Operation::Delete
            } else {
                Operation::Write
            },
            line_start: None,
            line_end: None,
        });
    }
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut command = std::process::Command::new("git");
    command.arg("-C").arg(root).args(args);
    let output =
        crate::git_process::bounded_output(command, Duration::from_millis(250), 256 * 1024)?;
    if !output.status.success() {
        bail!("Git preflight could not resolve integration paths");
    }
    Ok(output.stdout)
}

fn literal_git(command: &str) -> Option<(Vec<String>, usize)> {
    let words = shell_words::split(command).ok()?;
    if !words.first().is_some_and(|word| word == "git") {
        return None;
    }
    let mut index = 1;
    while words.get(index).is_some_and(|word| word == "-C") {
        words.get(index + 1)?;
        index += 2;
        if index > 17 {
            return None;
        }
    }
    Some((words, index))
}
