// @feature coordination
// @feature usage-analytics
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
// @boundary authenticated-client-evidence
use super::api::{
    validate_required_field, ServiceRequest, ToolHookPhase, MAX_AGENT_BYTES, MAX_SESSION_ID_BYTES,
};
use super::domain::{HookContext, PathIntent, ToolCompletion, ToolHookInput};
use super::usage_detection::MAX_DETECTED_SCRIPT_INVOCATIONS;
use super::{usage, workspace};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::ops::ControlFlow;
use std::path::{Component, Path};

const MAX_CHANGED_PATHS: usize = 2_048;
const MAX_CHANGED_PATH_BYTES: usize = 256 * 1_024;
const MAX_IDENTITY_BYTES: usize = 512;
const MAX_PATH_BYTES: usize = 4_096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientFilesystemEvidence {
    pub(crate) session_id: String,
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) changed_paths: Vec<PathIntent>,
    #[serde(default)]
    pub(crate) changed_paths_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) derived_usage: Option<ClientDerivedUsage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientDerivedUsage {
    #[serde(default)]
    pub(crate) scripts: Vec<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(crate) script_versions: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) repository_inventory: Option<crate::analytics::types::RepositoryInventory>,
}

pub(crate) struct ClientEvidencePlan {
    project_root: String,
    session_id: String,
    agent: String,
    usage: Option<(ToolHookInput, ToolCompletion)>,
}

impl ClientEvidencePlan {
    pub(crate) fn for_request(request: &ServiceRequest) -> Option<Self> {
        let (context, completion, input) = match request {
            ServiceRequest::ToolHook {
                phase: ToolHookPhase::After(completion),
                input,
            } => (&input.context, *completion, input),
            ServiceRequest::ToolHook {
                phase: ToolHookPhase::Before,
                input,
            } => {
                return Self::from_context(
                    &input.context,
                    Some((input.clone(), ToolCompletion::Failed)),
                );
            }
            _ => return None,
        };
        Self::from_context(context, Some((input.clone(), completion)))
    }

    fn from_context(
        context: &HookContext,
        usage: Option<(ToolHookInput, ToolCompletion)>,
    ) -> Option<Self> {
        Some(Self {
            project_root: context.project_root.clone()?,
            session_id: context.session_id.clone(),
            agent: context.agent.clone(),
            usage,
        })
    }

    pub(crate) fn capture(self) -> Option<ClientFilesystemEvidence> {
        let derived_usage = self.usage.and_then(|(input, completion)| {
            let mut evidence = usage::derive_filesystem_evidence(
                &input,
                completion,
                Path::new(&self.project_root),
            );
            evidence.repository_inventory = crate::analytics::inventory::capture_once(
                Path::new(&self.project_root),
                &input.context,
            );
            evidence.scripts.truncate(MAX_DETECTED_SCRIPT_INVOCATIONS);
            evidence.scripts.retain(|script| {
                validate_identity("derived script", script).is_ok()
                    && validate_relative_path(script).is_ok()
            });
            evidence.skill = evidence
                .skill
                .filter(|skill| validate_identity("derived skill", skill).is_ok());
            (!evidence.scripts.is_empty()
                || evidence.skill.is_some()
                || evidence.repository_inventory.is_some())
            .then_some(evidence)
        });
        let evidence = ClientFilesystemEvidence {
            session_id: self.session_id,
            agent: self.agent,
            changed_paths: Vec::new(),
            changed_paths_truncated: false,
            derived_usage,
        };
        evidence.validate().is_ok().then_some(evidence)
    }
}

#[derive(Default)]
pub(crate) struct ChangedPathPage {
    pub(crate) paths: Vec<PathIntent>,
    pub(crate) next_offset: usize,
    pub(crate) truncated: bool,
}

pub(crate) fn capture_changed_paths_page(project_root: &str, offset: usize) -> ChangedPathPage {
    workspace::observe_changed_paths(project_root)
        .map(|observation| page_changed_paths(&observation, offset))
        .unwrap_or_default()
}

fn page_changed_paths(
    observation: &workspace::ChangedPathObservation,
    offset: usize,
) -> ChangedPathPage {
    let mut total = 0_usize;
    observation.visit(|path, _| {
        if validate_relative_path(path).is_ok() {
            total += 1;
        }
        ControlFlow::Continue(())
    });
    if total == 0 {
        return ChangedPathPage::default();
    }
    let start = offset % total;
    let mut page = Vec::new();
    let mut path_bytes = 0;
    let filled = append_changed_path_range(observation, start, total, &mut page, &mut path_bytes);
    if !filled && start > 0 {
        append_changed_path_range(observation, 0, start, &mut page, &mut path_bytes);
    }
    let truncated = page.len() < total;
    ChangedPathPage {
        next_offset: if truncated {
            (start + page.len()) % total
        } else {
            0
        },
        paths: page,
        truncated,
    }
}

fn append_changed_path_range(
    observation: &workspace::ChangedPathObservation,
    range_start: usize,
    range_end: usize,
    page: &mut Vec<PathIntent>,
    path_bytes: &mut usize,
) -> bool {
    let mut valid_index = 0_usize;
    let mut filled = false;
    observation.visit(|path, operation| {
        if validate_relative_path(path).is_err() {
            return ControlFlow::Continue(());
        }
        let index = valid_index;
        valid_index += 1;
        if index < range_start || index >= range_end {
            return ControlFlow::Continue(());
        }
        if page.len() >= MAX_CHANGED_PATHS
            || path_bytes.saturating_add(path.len()) > MAX_CHANGED_PATH_BYTES
        {
            filled = true;
            return ControlFlow::Break(());
        }
        *path_bytes += path.len();
        page.push(PathIntent {
            path: path.to_owned(),
            operation,
            line_start: None,
            line_end: None,
        });
        ControlFlow::Continue(())
    });
    filled
}

impl ClientFilesystemEvidence {
    pub(crate) fn reconciliation(
        session_id: impl Into<String>,
        agent: impl Into<String>,
        changed_paths: Vec<PathIntent>,
        changed_paths_truncated: bool,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            agent: agent.into(),
            changed_paths,
            changed_paths_truncated,
            derived_usage: None,
        }
    }

    pub(crate) fn validate_for(&self, request: &ServiceRequest) -> Result<()> {
        let context = match request {
            ServiceRequest::ToolHook { input, .. } => &input.context,
            _ => bail!("client evidence requires a tool lifecycle request"),
        };
        if self.session_id != context.session_id || self.agent != context.agent {
            bail!("client evidence does not match the lifecycle caller");
        }
        self.validate()
    }

    pub(crate) fn validate_reconciliation(&self) -> Result<()> {
        if self.derived_usage.is_some() {
            bail!("standalone reconciliation cannot include derived usage");
        }
        self.validate()
    }

    fn validate(&self) -> Result<()> {
        validate_required_field(
            "client evidence session_id",
            &self.session_id,
            MAX_SESSION_ID_BYTES,
        )?;
        validate_required_field("client evidence agent", &self.agent, MAX_AGENT_BYTES)?;
        validate_changed_paths(&self.changed_paths)?;
        if let Some(usage) = &self.derived_usage {
            validate_derived_usage(usage)?;
        }
        Ok(())
    }
}

fn validate_changed_paths(paths: &[PathIntent]) -> Result<()> {
    if paths.len() > MAX_CHANGED_PATHS {
        bail!("client evidence contains too many changed paths");
    }
    for intent in paths {
        if intent.line_start.is_some() || intent.line_end.is_some() {
            bail!("client reconciliation paths must be file-wide");
        }
        validate_relative_path(&intent.path)?;
    }
    if paths.iter().map(|intent| intent.path.len()).sum::<usize>() > MAX_CHANGED_PATH_BYTES {
        bail!("client evidence changed paths exceed the aggregate byte limit");
    }
    Ok(())
}

fn validate_derived_usage(usage: &ClientDerivedUsage) -> Result<()> {
    if let Some(inventory) = &usage.repository_inventory {
        crate::analytics::inventory::validate("repository", &inventory.resources)?;
    }
    if usage.scripts.len() > MAX_DETECTED_SCRIPT_INVOCATIONS {
        bail!("client evidence contains too many derived scripts");
    }
    if usage.script_versions.len() > MAX_DETECTED_SCRIPT_INVOCATIONS
        || usage.script_versions.iter().any(|(name, version)| {
            !usage.scripts.contains(name)
                || version.len() != 64
                || !version.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        bail!("invalid script version evidence");
    }
    for script in &usage.scripts {
        validate_identity("derived script", script)?;
        validate_relative_path(script)?;
    }
    if let Some(skill) = &usage.skill {
        validate_identity("derived skill", skill)?;
    }
    Ok(())
}

fn validate_identity(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > MAX_IDENTITY_BYTES {
        bail!("client evidence {field} must contain 1 to {MAX_IDENTITY_BYTES} bytes");
    }
    if value.chars().any(char::is_control) {
        bail!("client evidence {field} cannot contain control characters");
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<()> {
    let path = Path::new(value);
    let valid = !value.is_empty()
        && value.len() <= MAX_PATH_BYTES
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
    if !valid {
        bail!("client evidence paths must be bounded repository-relative paths");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
