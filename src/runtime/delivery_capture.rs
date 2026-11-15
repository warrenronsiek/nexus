// @feature delivery-coordination
// @feature runtime
// @spec docs/features/delivery-coordination.md
// @spec docs/features/runtime.md
// @entrypoint capture
// @boundary local-git-metadata: bounded committed Git reads retain hashes, ranges and feature annotations without storing patch contents or inferring authorship.
use crate::work_history::GitRevision;
use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

const MAX_FILES: usize = 128;
const MAX_HUNKS: usize = 2048;
const MAX_OUTPUT: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapturedContribution {
    pub repository: String,
    pub base: GitRevision,
    pub head: GitRevision,
    pub branch: Option<String>,
    pub worktree: String,
    pub files: Vec<CapturedFile>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapturedFile {
    pub path: String,
    pub hunks: Vec<CapturedHunk>,
    pub features: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapturedHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub fingerprint: String,
}

pub fn capture(
    root: &Path,
    repository: &str,
    base: &str,
    head: &str,
) -> Result<CapturedContribution> {
    anyhow::ensure!(
        !repository.trim().is_empty()
            && repository.len() <= 512
            && !repository.chars().any(char::is_control),
        "invalid repository identity"
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let (root, format) = checkout(root, deadline)?;
    let context = CaptureGit {
        root: &root,
        repository,
        format,
        deadline,
    };
    let base = context.revision(base)?;
    let head = context.revision(head)?;
    let branch = git_text(
        &root,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        deadline,
    )
    .ok();
    let files = context.files(&base.commit, &head.commit)?;
    Ok(CapturedContribution {
        repository: repository.into(),
        base,
        head,
        branch,
        worktree: root.to_string_lossy().into_owned(),
        files,
    })
}

fn checkout(root: &Path, deadline: Instant) -> Result<(std::path::PathBuf, String)> {
    let root = root
        .canonicalize()
        .context("contribution checkout unavailable")?;
    let format = git_text(&root, &["rev-parse", "--show-object-format"], deadline)?;
    Ok((root, format))
}

struct CaptureGit<'a> {
    root: &'a Path,
    repository: &'a str,
    format: String,
    deadline: Instant,
}
impl CaptureGit<'_> {
    fn revision(&self, value: &str) -> Result<GitRevision> {
        anyhow::ensure!(
            value.len() <= 1024 && !value.chars().any(char::is_control),
            "invalid revision selector"
        );
        let commit = git_text(
            self.root,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{value}^{{commit}}"),
            ],
            self.deadline,
        )?;
        anyhow::ensure!(
            matches!(commit.len(), 40 | 64) && commit.bytes().all(|b| b.is_ascii_hexdigit()),
            "Git returned invalid commit identity"
        );
        Ok(GitRevision {
            repository: self.repository.into(),
            object_format: self.format.clone(),
            commit,
        })
    }

    fn files(&self, base: &str, head: &str) -> Result<Vec<CapturedFile>> {
        let raw = git(
            self.root,
            &[
                "diff",
                "--name-status",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                "-z",
                base,
                head,
                "--",
            ],
            self.deadline,
            MAX_OUTPUT,
        )?;
        let mut fields = raw.split(|b| *b == 0).filter(|v| !v.is_empty());
        let mut files = Vec::new();
        let mut hunk_count = 0;
        while let Some(metadata) = fields.next() {
            anyhow::ensure!(
                files.len() < MAX_FILES,
                "contribution exceeds 128 changed files; register smaller explicit contributions"
            );
            let path =
                std::str::from_utf8(fields.next().context("missing Git changed path")?)?.to_owned();
            let file = self.file(metadata, path, base, head)?;
            hunk_count += file.hunks.len();
            anyhow::ensure!(
                hunk_count <= MAX_HUNKS,
                "contribution exceeds 2048 changed hunks"
            );
            files.push(file);
        }
        Ok(files)
    }

    fn file(&self, metadata: &[u8], path: String, base: &str, head: &str) -> Result<CapturedFile> {
        let metadata = std::str::from_utf8(metadata)?;
        anyhow::ensure!(
            path.len() <= 4096 && !path.chars().any(char::is_control),
            "changed path exceeds contribution contract"
        );
        let feature_revision = match metadata {
            "D" => base,
            "A" | "M" | "T" => head,
            _ => anyhow::bail!("unsupported Git change metadata"),
        };
        let patch = git(
            self.root,
            &[
                "diff",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                "--unified=0",
                base,
                head,
                "--",
                &path,
            ],
            self.deadline,
            MAX_OUTPUT,
        )?;
        let hunks = parse_hunks(&patch)?;
        let feature_source = git(
            self.root,
            &["show", &format!("{feature_revision}:{path}")],
            self.deadline,
            128 * 1024,
        );
        let features = feature_source
            .map(|bytes| feature_annotations(&bytes))
            .unwrap_or_default();
        Ok(CapturedFile {
            path,
            hunks,
            features,
        })
    }
}

fn parse_hunks(bytes: &[u8]) -> Result<Vec<CapturedHunk>> {
    let mut result = Vec::new();
    let mut current: Option<(CapturedHunk, blake3::Hasher)> = None;
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if line.starts_with(b"@@ ") {
            if let Some((mut hunk, hash)) = current.take() {
                hunk.fingerprint = hash.finalize().to_hex().to_string();
                result.push(hunk);
            }
            let header = std::str::from_utf8(line)?;
            let mut parts = header.split_ascii_whitespace();
            parts.next();
            let (old_start, old_lines) =
                range(parts.next().context("missing old hunk range")?, '-')?;
            let (new_start, new_lines) =
                range(parts.next().context("missing new hunk range")?, '+')?;
            current = Some((
                CapturedHunk {
                    old_start,
                    old_lines,
                    new_start,
                    new_lines,
                    fingerprint: String::new(),
                },
                blake3::Hasher::new(),
            ));
        } else if let Some((_, hash)) = &mut current {
            hash.update(line);
        }
    }
    if let Some((mut hunk, hash)) = current {
        hunk.fingerprint = hash.finalize().to_hex().to_string();
        result.push(hunk);
    }
    Ok(result)
}
fn range(value: &str, sign: char) -> Result<(u32, u32)> {
    let value = value.strip_prefix(sign).context("invalid Git hunk range")?;
    let (start, lines) = value.split_once(',').unwrap_or((value, "1"));
    Ok((start.parse()?, lines.parse()?))
}
fn feature_annotations(bytes: &[u8]) -> Vec<String> {
    let source = String::from_utf8_lossy(bytes);
    let mut tags = BTreeSet::new();
    for line in source.lines().take(64) {
        if let Some((_, suffix)) = line.split_once("@feature ") {
            if let Some(tag) = suffix.split_ascii_whitespace().next().filter(|tag| {
                tag.len() <= 128
                    && tag
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            }) {
                tags.insert(tag.to_owned());
            }
        }
    }
    tags.into_iter().collect()
}
fn git_text(root: &Path, args: &[&str], deadline: Instant) -> Result<String> {
    Ok(String::from_utf8(git(root, args, deadline, 8192)?)?
        .trim()
        .to_owned())
}
fn git(root: &Path, args: &[&str], deadline: Instant, limit: usize) -> Result<Vec<u8>> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .context("contribution capture deadline exceeded")?;
    let mut command = Command::new("git");
    command
        .arg("--literal-pathspecs")
        .arg("-C")
        .arg(root)
        .args(args);
    let output =
        crate::git_process::bounded_output(command, remaining.min(Duration::from_secs(2)), limit)?;
    anyhow::ensure!(
        output.status.success(),
        "Git contribution metadata unavailable"
    );
    Ok(output.stdout)
}
