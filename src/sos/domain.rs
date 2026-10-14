// @feature sos
// @spec docs/features/sos.md
// @boundary dynamic-json
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::str::FromStr;

pub const MAX_SOS_MESSAGE_BYTES: usize = 16 * 1024;
pub const MAX_SOS_TRACE_BYTES: usize = 64 * 1024;
pub const MAX_SOS_METADATA_BYTES: usize = 32 * 1024;
pub const MAX_SOS_TEXT_BYTES: usize = 8 * 1024;
pub const MAX_SOS_TITLE_BYTES: usize = 256;
pub const MAX_SOS_IDENTITY_BYTES: usize = 256;
const MAX_SOS_ENVIRONMENT_BYTES: usize = 512;
const MAX_SOS_METADATA_DEPTH: usize = 16;

macro_rules! stored_enum {
    ($type:ty, $kind:literal, {$($variant:ident => $value:literal),+ $(,)?}) => {
        impl std::fmt::Display for $type {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(match self { $(Self::$variant => $value),+ })
            }
        }

        impl FromStr for $type {
            type Err = String;

            fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    other => Err(format!("unknown {} {other}", $kind)),
                }
            }
        }
    };
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(transparent)]
pub struct SosMetadata(BTreeMap<String, Value>);

impl SosMetadata {
    pub fn as_map(&self) -> &BTreeMap<String, Value> {
        &self.0
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let encoded = serde_json::to_vec(&self.0).context("encode SOS metadata")?;
        if encoded.len() > MAX_SOS_METADATA_BYTES {
            bail!("SOS metadata exceeds {MAX_SOS_METADATA_BYTES}-byte limit");
        }
        for value in self.0.values() {
            validate_metadata_depth(value, 1)?;
        }
        Ok(())
    }
}

fn validate_metadata_depth(value: &Value, depth: usize) -> Result<()> {
    if depth > MAX_SOS_METADATA_DEPTH {
        bail!("SOS metadata exceeds {MAX_SOS_METADATA_DEPTH}-level nesting limit");
    }
    match value {
        Value::Array(values) => {
            for value in values {
                validate_metadata_depth(value, depth + 1)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_metadata_depth(value, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

impl TryFrom<Value> for SosMetadata {
    type Error = anyhow::Error;

    fn try_from(value: Value) -> Result<Self> {
        let Value::Object(values) = value else {
            bail!("SOS metadata must be a JSON object");
        };
        let metadata = Self(values.into_iter().collect());
        metadata.validate()?;
        Ok(metadata)
    }
}

impl<'de> Deserialize<'de> for SosMetadata {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let metadata = Self(BTreeMap::<String, Value>::deserialize(deserializer)?);
        metadata.validate().map_err(serde::de::Error::custom)?;
        Ok(metadata)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SosActor {
    pub agent: String,
    pub session_id: String,
    pub model_id: Option<String>,
    pub config_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SosReportDraft {
    pub message: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub environment: String,
    pub version: String,
    pub model: String,
    pub trace: String,
    #[serde(default)]
    pub metadata: SosMetadata,
}

impl SosReportDraft {
    pub(crate) fn normalized(mut self) -> Result<Self> {
        self.normalize_content()?;
        self.normalize_dimensions()?;
        Ok(self)
    }

    fn normalize_content(&mut self) -> Result<()> {
        self.message = required_text("SOS message", &self.message, MAX_SOS_MESSAGE_BYTES)?;
        self.trace = bounded_text("SOS trace", &self.trace, MAX_SOS_TRACE_BYTES)?;
        self.metadata.validate()
    }

    fn normalize_dimensions(&mut self) -> Result<()> {
        self.kind = required_text("SOS type", &self.kind, MAX_SOS_IDENTITY_BYTES)?;
        self.environment = required_text(
            "SOS environment",
            &self.environment,
            MAX_SOS_ENVIRONMENT_BYTES,
        )?;
        self.version = required_text("SOS version", &self.version, MAX_SOS_IDENTITY_BYTES)?;
        self.model = required_text("SOS model", &self.model, MAX_SOS_IDENTITY_BYTES)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SosReport {
    pub id: String,
    pub project_id: String,
    pub issue_id: Option<String>,
    pub message: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub environment: String,
    pub version: String,
    pub model: String,
    pub trace: String,
    pub metadata: SosMetadata,
    pub reporter: SosActor,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SosIssueStatus {
    Open,
    Resolved,
    Deprecated,
}

stored_enum!(SosIssueStatus, "SOS issue status", {
    Open => "open",
    Resolved => "resolved",
    Deprecated => "deprecated",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SosIssue {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub summary: String,
    pub status: SosIssueStatus,
    pub resolution: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub first_reported_at: DateTime<Utc>,
    pub last_reported_at: DateTime<Utc>,
    pub importance_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SosIssueWindow {
    pub issues: Vec<SosIssue>,
    pub untriaged_report_count: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SosTriageCandidate {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub summary: String,
    pub status: SosIssueStatus,
    pub last_reported_at: DateTime<Utc>,
    pub importance_count: u64,
}

impl From<&SosIssue> for SosTriageCandidate {
    fn from(issue: &SosIssue) -> Self {
        Self {
            id: issue.id.clone(),
            project_id: issue.project_id.clone(),
            title: issue.title.clone(),
            summary: issue.summary.clone(),
            status: issue.status,
            last_reported_at: issue.last_reported_at,
            importance_count: issue.importance_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SosTriageDecision {
    Create {
        title: String,
        summary: String,
        rationale: String,
    },
    Link {
        issue_id: String,
        title: String,
        summary: String,
        rationale: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SosTriageOutcome {
    Created,
    Linked,
    Failed,
}

stored_enum!(SosTriageOutcome, "SOS triage outcome", {
    Created => "created",
    Linked => "linked",
    Failed => "failed",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SosTriageAttempt {
    pub id: String,
    pub project_id: String,
    pub report_id: String,
    pub issue_id: Option<String>,
    pub outcome: SosTriageOutcome,
    pub provider: String,
    pub model_id: Option<String>,
    pub rationale: String,
    pub generated_title: Option<String>,
    pub generated_summary: Option<String>,
    pub config_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SosIssueTransition {
    Resolved,
    Deprecated,
    Reopened,
}

stored_enum!(SosIssueTransition, "SOS issue transition", {
    Resolved => "resolved",
    Deprecated => "deprecated",
    Reopened => "reopened",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SosIssueLifecycleEvent {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub transition: SosIssueTransition,
    pub reason: String,
    pub actor: SosActor,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SosIssueDetail {
    pub issue: SosIssue,
    pub reports: Vec<SosReport>,
    pub reports_truncated: bool,
    pub triage_attempts: Vec<SosTriageAttempt>,
    pub triage_attempts_truncated: bool,
    pub lifecycle_events: Vec<SosIssueLifecycleEvent>,
    pub lifecycle_events_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SosRegistry {
    pub generated_at: DateTime<Utc>,
    pub truncated: bool,
    pub issues: Vec<SosIssue>,
    pub reports: Vec<SosReport>,
    pub triage_attempts: Vec<SosTriageAttempt>,
    pub lifecycle_events: Vec<SosIssueLifecycleEvent>,
}

pub(crate) fn validate_actor(actor: &SosActor) -> Result<()> {
    required_text("SOS actor agent", &actor.agent, MAX_SOS_IDENTITY_BYTES)?;
    required_text(
        "SOS actor session_id",
        &actor.session_id,
        MAX_SOS_IDENTITY_BYTES * 2,
    )?;
    if let Some(model) = actor.model_id.as_deref() {
        required_text("SOS actor model_id", model, MAX_SOS_IDENTITY_BYTES)?;
    }
    required_text(
        "SOS actor config_hash",
        &actor.config_hash,
        MAX_SOS_IDENTITY_BYTES,
    )?;
    Ok(())
}

pub(crate) fn required_text(field: &str, value: &str, maximum: usize) -> Result<String> {
    if value.len() > maximum {
        bail!("{field} exceeds {maximum}-byte limit");
    }
    let value = value.trim();
    if value.is_empty() {
        bail!("{field} cannot be empty");
    }
    Ok(value.to_owned())
}

fn bounded_text(field: &str, value: &str, maximum: usize) -> Result<String> {
    if value.len() > maximum {
        bail!("{field} exceeds {maximum}-byte limit");
    }
    Ok(value.trim().to_owned())
}
