// @feature delivery-coordination
// @spec docs/features/delivery-coordination.md
use super::validation::{git, text};
use crate::work_history::GitRevision;
use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentKey {
    pub repository: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReleaseRequirements {
    pub id: String,
    pub revision: GitRevision,
    pub requires: Vec<EnvironmentKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentInventory {
    pub id: String,
    pub repository: String,
    pub environment: String,
    pub keys: Vec<String>,
    pub complete: bool,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentAdvisory {
    pub id: String,
    pub environment: String,
    pub title: String,
    pub description: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DependencyStatus {
    Present,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DependencyAssessment {
    pub requirement: EnvironmentKey,
    pub status: DependencyStatus,
    pub inventory_id: Option<String>,
}

impl ReleaseRequirements {
    pub(crate) fn validate(&self) -> Result<()> {
        git(&self.revision)?;
        if self.requires.len() > 128 {
            bail!("at most 128 release requirements are supported");
        }
        let mut seen = std::collections::HashSet::new();
        for requirement in &self.requires {
            text(&requirement.repository, 512)?;
            text(&requirement.key, 256)?;
            if !seen.insert((&requirement.repository, &requirement.key)) {
                bail!("duplicate release requirement");
            }
        }
        Ok(())
    }
}

impl EnvironmentInventory {
    pub(crate) fn validate(&self, occurred_at: DateTime<Utc>) -> Result<()> {
        text(&self.repository, 512)?;
        text(&self.environment, 256)?;
        if self.keys.len() > 256 || self.expires_at <= occurred_at {
            bail!("inventory requires a later expiration and at most 256 keys");
        }
        self.keys.iter().try_for_each(|key| text(key, 256))
    }
}

impl EnvironmentAdvisory {
    pub(crate) fn validate(&self, occurred_at: DateTime<Utc>) -> Result<()> {
        text(&self.environment, 256)?;
        text(&self.title, 512)?;
        text(&self.description, 4096)?;
        if self.expires_at <= occurred_at {
            bail!("advisory expiration must follow its observation");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PromotionReadiness {
    pub environment: String,
    pub candidate: GitRevision,
    pub requirements_id: Option<String>,
    pub missing: usize,
    pub unknown: usize,
}
