// @feature delivery-coordination
// @spec docs/features/delivery-coordination.md
use crate::work_history::WorkActor;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GithubSource {
    pub owner: String,
    pub repository: String,
    pub number: u64,
}

impl GithubSource {
    pub fn validate(&self) -> anyhow::Result<()> {
        for value in [&self.owner, &self.repository] {
            anyhow::ensure!(
                !value.is_empty()
                    && value.len() <= 100
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
                "invalid GitHub repository identity"
            );
            anyhow::ensure!(
                value != "." && value != "..",
                "invalid GitHub repository identity"
            );
        }
        anyhow::ensure!(self.number > 0, "pull-request number must be positive");
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeliveryWatchRegistration {
    pub id: String,
    pub pull_request_id: String,
    pub repository_id: String,
    pub source: GithubSource,
    pub interval_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promotion_environment: Option<String>,
}

impl DeliveryWatchRegistration {
    pub fn validate(&self) -> anyhow::Result<()> {
        for value in [&self.id, &self.pull_request_id, &self.repository_id] {
            anyhow::ensure!(
                !value.trim().is_empty()
                    && value.len() <= 512
                    && !value.chars().any(char::is_control),
                "invalid delivery watch identity"
            );
        }
        anyhow::ensure!(
            (30..=86400).contains(&self.interval_seconds),
            "watch interval must be between 30 and 86400 seconds"
        );
        if let Some(environment) = &self.promotion_environment {
            super::validation::text(environment, 256)?;
        }
        self.source.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeliveryWatch {
    pub registration: DeliveryWatchRegistration,
    pub project_id: String,
    pub provenance: WorkActor,
    pub generation: String,
    pub enabled: bool,
    pub next_poll_at: DateTime<Utc>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub last_digest: Option<String>,
    pub provider_updated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promotion_readiness: Option<super::PromotionReadiness>,
}

pub enum DeliveryWatchOutcome {
    Observed {
        snapshot: Box<super::PullRequestSnapshot>,
        provider_updated_at: DateTime<Utc>,
        digest: String,
    },
    Failed {
        reason: String,
    },
}
