// @feature usage-analytics
// @spec docs/features/usage-analytics.md
//! Skill manifest text captured beside the checkout when an agent reads a `SKILL.md`.
//!
//! The text is the instruction document itself, so it is retained verbatim for people to read.
//! Only the bounded manifest and a display path (repository-relative, or starting at the folder
//! that owns `skills/`) cross the wire; absolute client paths never do.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

pub const MAX_SKILL_DOCUMENT_BYTES: usize = 64 * 1024;
const MAX_SOURCE_PATH_BYTES: usize = 4 * 1024;
const MAX_DESCRIPTION_CHARS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDocument {
    pub content: String,
    pub source_path: String,
}

impl SkillDocument {
    pub fn validate(&self) -> Result<()> {
        if self.content.len() > MAX_SKILL_DOCUMENT_BYTES {
            bail!("skill document exceeds {MAX_SKILL_DOCUMENT_BYTES} bytes");
        }
        let path = Path::new(&self.source_path);
        let relative = !self.source_path.is_empty()
            && self.source_path.len() <= MAX_SOURCE_PATH_BYTES
            && !self.source_path.chars().any(char::is_control)
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_)));
        if !relative {
            bail!("skill document source_path must be a bounded relative display path");
        }
        Ok(())
    }

    pub fn content_hash(&self) -> String {
        format!("{:x}", Sha256::digest(self.content.as_bytes()))
    }

    pub fn description(&self) -> Option<String> {
        frontmatter_field(&self.content, "description")
    }
}

/// Reads one top-level YAML frontmatter scalar, including folded or literal block values.
pub fn frontmatter_field(content: &str, key: &str) -> Option<String> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut frontmatter = lines.take_while(|line| line.trim() != "---").peekable();
    while let Some(line) = frontmatter.next() {
        let Some(value) = line
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix(':'))
            .map(str::trim)
        else {
            continue;
        };
        let value = match value.chars().next() {
            Some(style @ ('|' | '>')) => {
                let mut block = Vec::new();
                while let Some(next) = frontmatter
                    .next_if(|next| next.trim().is_empty() || next.starts_with([' ', '\t']))
                {
                    block.push(next.trim());
                }
                block.join(if style == '|' { "\n" } else { " " })
            }
            _ => value.trim_matches(['\'', '"']).to_owned(),
        };
        let value = value.trim();
        return (!value.is_empty()).then(|| value.chars().take(MAX_DESCRIPTION_CHARS).collect());
    }
    None
}

/// A privacy-safe label for a canonical manifest path.
pub fn display_path(manifest: &Path, project_root: &Path) -> Option<String> {
    let relative = match project_root
        .canonicalize()
        .ok()
        .and_then(|root| manifest.strip_prefix(root).ok().map(Path::to_path_buf))
    {
        Some(relative) => relative,
        None => {
            let components: Vec<_> = manifest.components().collect();
            let skills = components
                .iter()
                .rposition(|part| part.as_os_str() == "skills")?;
            let owner = skills
                .checked_sub(1)
                .filter(|index| matches!(components[*index], Component::Normal(_)))
                .unwrap_or(skills);
            components[owner..].iter().collect()
        }
    };
    let display = relative.to_str()?.to_owned();
    let document = SkillDocument {
        content: String::new(),
        source_path: display,
    };
    document.validate().ok().map(|()| document.source_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_fields_read_plain_quoted_and_block_scalars() {
        let manifest = "---\nname: 'tdd'\ndescription: >\n  Test first.\n  Then refactor.\nmetadata:\n  description: nested\n---\n# Body\ndescription: body\n";
        assert_eq!(frontmatter_field(manifest, "name").as_deref(), Some("tdd"));
        assert_eq!(
            frontmatter_field(manifest, "description").as_deref(),
            Some("Test first. Then refactor.")
        );
        let literal = "---\ndescription: |\n  one\n  two\n---\n";
        assert_eq!(
            frontmatter_field(literal, "description").as_deref(),
            Some("one\ntwo")
        );
        assert_eq!(frontmatter_field("# No frontmatter\n", "name"), None);
        assert_eq!(frontmatter_field("---\nname: \"\"\n---\n", "name"), None);
    }

    #[test]
    fn display_paths_never_expose_absolute_client_locations() {
        let checkout = tempfile::tempdir().unwrap();
        let inside = checkout
            .path()
            .canonicalize()
            .unwrap()
            .join("skills/x/SKILL.md");
        assert_eq!(
            display_path(&inside, checkout.path()).as_deref(),
            Some("skills/x/SKILL.md")
        );
        let home = Path::new("/home/someone/.codex/skills/tdd/SKILL.md");
        assert_eq!(
            display_path(home, checkout.path()).as_deref(),
            Some(".codex/skills/tdd/SKILL.md")
        );
        let root = Path::new("/skills/tdd/SKILL.md");
        assert_eq!(
            display_path(root, checkout.path()).as_deref(),
            Some("skills/tdd/SKILL.md")
        );
    }

    #[test]
    fn documents_are_bounded_and_hashed_with_sha256() {
        let document = SkillDocument {
            content: "abc".into(),
            source_path: ".codex/skills/x/SKILL.md".into(),
        };
        assert_eq!(
            document.content_hash(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(document.validate().is_ok());
        for (content, source_path) in [
            (
                "x".repeat(MAX_SKILL_DOCUMENT_BYTES + 1),
                "skills/x/SKILL.md",
            ),
            (String::new(), "/home/someone/skills/x/SKILL.md"),
            (String::new(), "../skills/x/SKILL.md"),
            (String::new(), ""),
        ] {
            let invalid = SkillDocument {
                content,
                source_path: source_path.into(),
            };
            assert!(invalid.validate().is_err(), "{source_path}");
        }
    }
}
