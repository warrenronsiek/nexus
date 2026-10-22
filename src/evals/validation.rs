// @feature evals
// @spec docs/features/evals.md
use super::*;
use anyhow::{bail, Result};
mod metrics;
mod payloads;
mod relationships;

pub const MAX_CHUNK_BYTES: usize = 256 * 1024;
pub const MAX_BLOB_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const MAX_RECORD_BYTES: usize = 64 * 1024;
pub(crate) const MAX_BUNDLE_RECORDS: usize = 64;

pub fn digest_is_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn identity(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        bail!("{name} must be nonempty, control-free, and at most 512 bytes");
    }
    Ok(())
}

pub(super) fn text(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 16 * 1024 || value.contains('\0') {
        bail!("{name} must contain 1 to 16384 bytes without NUL");
    }
    Ok(())
}

pub(super) fn oid(value: &str, format: ObjectFormat) -> Result<()> {
    let length = match format {
        ObjectFormat::Sha1 => 40,
        ObjectFormat::Sha256 => 64,
    };
    if value.len() != length
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("Git identity must be a full lowercase object ID in the declared format");
    }
    Ok(())
}

pub(super) fn path(value: &str) -> Result<()> {
    identity("path", value)?;
    if value.starts_with('/')
        || value.contains('\\')
        || value
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        bail!("artifact paths must be repository relative without traversal");
    }
    Ok(())
}

pub(crate) fn validate_backing(backing: &ArtifactBacking) -> Result<()> {
    match backing {
        ArtifactBacking::GitFile {
            repository,
            commit,
            object_format,
            path,
            blob_oid,
            entry_mode,
        } => {
            validate_git_coordinates(repository, commit, *object_format, path)?;
            validate_blob_details(blob_oid.as_deref(), *object_format, entry_mode)
        }
        ArtifactBacking::GitLfs {
            repository,
            commit,
            path,
            oid,
            ..
        } => {
            let format = if commit.len() == 40 {
                ObjectFormat::Sha1
            } else {
                ObjectFormat::Sha256
            };
            validate_git_coordinates(repository, commit, format, path)?;
            validate_digest(oid)
        }
        ArtifactBacking::StoredBlob { digest } => validate_digest(digest),
    }
}

fn validate_git_coordinates(
    repository: &str,
    commit: &str,
    format: ObjectFormat,
    name: &str,
) -> Result<()> {
    identity("repository", repository)?;
    oid(commit, format)?;
    path(name)
}

fn validate_blob_details(blob: Option<&str>, format: ObjectFormat, mode: &str) -> Result<()> {
    if let Some(blob) = blob {
        oid(blob, format)?;
    }
    if !["100644", "100755", "120000"].contains(&mode) {
        bail!("Git file mode must identify a regular file or symbolic link");
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<()> {
    if !digest_is_valid(value) {
        bail!("content must have a SHA-256 identity");
    }
    Ok(())
}

pub(crate) struct Reference {
    pub link: EvalLink,
    pub expected: Option<EvalKind>,
}

pub(super) fn link(
    refs: &mut Vec<Reference>,
    relation: &str,
    id: &str,
    expected: Option<EvalKind>,
) {
    refs.push(Reference {
        link: EvalLink {
            relation: relation.into(),
            target_id: id.into(),
        },
        expected,
    });
}
pub(super) fn links(
    refs: &mut Vec<Reference>,
    relation: &str,
    ids: &[String],
    expected: Option<EvalKind>,
) {
    for id in ids {
        link(refs, relation, id, expected);
    }
}
pub(super) fn optional(
    refs: &mut Vec<Reference>,
    relation: &str,
    id: &Option<String>,
    expected: Option<EvalKind>,
) {
    if let Some(id) = id {
        link(refs, relation, id, expected);
    }
}

impl EvalBody {
    pub fn revision(&self) -> Option<&Revision> {
        match self {
            Self::Repository(v) => Some(&v.revision),
            Self::Model(v) => Some(&v.revision),
            Self::Case(v) => Some(&v.revision),
            Self::Suite(v) => Some(&v.revision),
            Self::Grader(v) => Some(&v.revision),
            Self::Target(v) => Some(&v.revision),
            Self::Metric(v) => Some(&v.revision),
            _ => None,
        }
    }

    pub(crate) fn references(&self) -> Vec<Reference> {
        let mut refs = Vec::new();
        if let Some(revision) = self.revision() {
            link(
                &mut refs,
                "motivation",
                &revision.change_id,
                Some(EvalKind::Change),
            );
            links(
                &mut refs,
                "parent_revision",
                &revision.parents,
                Some(self.kind()),
            );
        }
        self.append_references(&mut refs);
        refs
    }
}

impl EvalRecordInput {
    pub fn validate(&self) -> Result<()> {
        identity("record id", &self.id)?;
        text("record title", &self.title)?;
        if self.title.len() > 1024 {
            bail!("record title exceeds 1024 bytes");
        }
        if serde_json::to_vec(self)?.len() > MAX_RECORD_BYTES {
            bail!("eval record exceeds 65536 bytes; use artifact references");
        }
        if let Some(rev) = self.body.revision() {
            identity("revision identity", &rev.identity)?;
        }
        let refs = self.body.references();
        if refs.len() > 128 {
            bail!("eval record exceeds 128 direct references");
        }
        for reference in &refs {
            identity("referenced id", &reference.link.target_id)?;
        }
        self.body.validate_payload()
    }
}

pub(crate) trait RecordReferences {
    fn append_references(&self, refs: &mut Vec<Reference>);
}
pub(crate) trait ValidatePayload {
    fn validate_payload(&self) -> Result<()>;
}
