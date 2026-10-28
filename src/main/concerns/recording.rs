// @feature code-concerns
// @feature runtime
// @spec docs/features/code-concerns.md
// @spec docs/features/runtime.md
// @boundary caller-concern-json: bounded explicit JSON supplies typed locations, evidence and records
use super::ConcernCliCommand;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use clap::Args;
use nexus::code_concerns::*;
use serde::{de::DeserializeOwned, Deserialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Default, Args)]
pub(crate) struct ObservationArgs {
    #[arg(long)]
    event_id: Option<String>,
    /// Source failure time; omission leaves it unknown.
    #[arg(long)]
    occurred_at: Option<DateTime<Utc>>,
    /// Reporter observation time; pin this when retrying a fixed event ID.
    #[arg(long)]
    observed_at: Option<DateTime<Utc>>,
    #[arg(long)]
    host_session: Option<String>,
    #[arg(long)]
    work_session: Option<String>,
    #[arg(long)]
    activity: Option<String>,
    #[arg(long)]
    repository: Option<String>,
}
#[derive(Debug, Default, Args)]
struct EncounterArgs {
    /// A ConcernEncounter JSON object. Additional flags append/override its details.
    #[arg(long)]
    encounter_json: Option<String>,
    /// Repeat for structured ConcernAreaAttribution JSON objects.
    #[arg(long)]
    area_json: Vec<String>,
    /// Source/repository/execution_attempt/failure_id JSON object.
    #[arg(long)]
    source_json: Option<String>,
    #[arg(long)]
    symptom: Option<String>,
    #[arg(long)]
    consequence: Option<String>,
    #[arg(long)]
    workaround: Option<String>,
}
#[derive(Debug, Args)]
pub(crate) struct ReportArgs {
    title: String,
    #[arg(long)]
    description: String,
    #[arg(long)]
    id: Option<String>,
    #[arg(long)]
    hypothesis: Option<String>,
    #[arg(long)]
    impact: Option<String>,
    #[command(flatten)]
    observation: ObservationArgs,
    #[command(flatten)]
    encounter: EncounterArgs,
}
#[derive(Debug, Args)]
pub(crate) struct UpvoteArgs {
    concern_id: String,
    #[arg(long)]
    note: Option<String>,
    #[arg(long)]
    fix_relation: Option<String>,
    #[command(flatten)]
    observation: ObservationArgs,
    #[command(flatten)]
    encounter: EncounterArgs,
}
#[derive(Debug, Args)]
pub(crate) struct RemedyArgs {
    concern_id: String,
    #[arg(long)]
    what: String,
    #[arg(long)]
    why: String,
    #[arg(long)]
    outcome: String,
    #[arg(long)]
    encounter_id: Vec<String>,
    #[arg(long)]
    evidence_json: Vec<String>,
    #[command(flatten)]
    observation: ObservationArgs,
}
#[derive(Debug, Args)]
pub(crate) struct ReviewArgs {
    concern_id: String,
    #[arg(long)]
    hypothesis: String,
    #[arg(long)]
    rationale: String,
    #[arg(long)]
    alternative: Vec<String>,
    #[arg(long)]
    encounter_id: Vec<String>,
    #[arg(long)]
    evidence_json: Vec<String>,
    #[command(flatten)]
    observation: ObservationArgs,
}
#[derive(Debug, Args)]
pub(crate) struct EvidenceArgs {
    concern_id: String,
    #[arg(long)]
    note: String,
    #[arg(long)]
    encounter_id: Option<String>,
    #[arg(long)]
    evidence_json: Vec<String>,
    #[command(flatten)]
    observation: ObservationArgs,
}
#[derive(Debug, Args)]
pub(crate) struct ResolveArgs {
    concern_id: String,
    #[arg(long)]
    reason: String,
    #[arg(long)]
    fixing_location_json: Option<String>,
    #[arg(long)]
    evidence_json: Vec<String>,
    #[command(flatten)]
    observation: ObservationArgs,
}
#[derive(Debug, Args)]
pub(crate) struct ReopenArgs {
    concern_id: String,
    #[arg(long)]
    reason: String,
    #[arg(long)]
    evidence_json: Vec<String>,
    #[command(flatten)]
    observation: ObservationArgs,
}
#[derive(Debug, Args)]
pub(crate) struct RespondArgs {
    concern_id: String,
    proposal_id: String,
    #[arg(long)]
    token_file: PathBuf,
    #[arg(long)]
    decision: String,
    #[arg(long)]
    reason: String,
    #[command(flatten)]
    observation: ObservationArgs,
}

impl ObservationArgs {
    fn build(self, concern_id: String, body: ConcernEvent) -> ConcernObservationInput {
        ConcernObservationInput {
            id: self.event_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            concern_id,
            occurred_at: self.occurred_at,
            observed_at: self.observed_at.unwrap_or_else(Utc::now),
            context: ConcernContext {
                host_session_id: self
                    .host_session
                    .or_else(|| std::env::var("NEXUS_HOST_SESSION_ID").ok()),
                work_session_id: self
                    .work_session
                    .or_else(|| std::env::var("NEXUS_WORK_SESSION_ID").ok()),
                activity_id: self
                    .activity
                    .or_else(|| std::env::var("NEXUS_PARENT_ACTIVITY_ID").ok()),
                repository: self.repository,
            },
            body,
        }
    }
}
impl EncounterArgs {
    fn build(self) -> Result<ConcernEncounter> {
        let mut encounter: ConcernEncounter = self
            .encounter_json
            .as_deref()
            .map(parse_json)
            .transpose()?
            .unwrap_or_default();
        encounter.areas.extend(
            self.area_json
                .iter()
                .map(|v| parse_json(v))
                .collect::<Result<Vec<ConcernAreaAttribution>>>()?,
        );
        if let Some(source) = self.source_json {
            encounter.source = Some(parse_json(&source)?)
        }
        if self.symptom.is_some() {
            encounter.symptom = self.symptom
        }
        if self.consequence.is_some() {
            encounter.consequence = self.consequence
        }
        if self.workaround.is_some() {
            encounter.workaround = self.workaround
        }
        Ok(encounter)
    }
}

#[path = "recording/actions.rs"]
mod actions;
pub(super) use actions::action;

fn evidence(values: Vec<String>) -> Result<Vec<ConcernEvidence>> {
    values.iter().map(|v| parse_json(v)).collect()
}
pub(super) fn json_enum<T: DeserializeOwned>(value: &str) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(value.to_owned())).map_err(Into::into)
}
fn parse_json<T: DeserializeOwned>(value: &str) -> Result<T> {
    if value.len() > 16 * 1024 {
        bail!("concern JSON value exceeds 16 KiB")
    }
    serde_json::from_str(value).context("decode typed concern JSON")
}

pub(super) fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let reader: Box<dyn Read> = if path == Path::new("-") {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(path)?)
    };
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("concern input exceeds {limit} bytes")
    }
    Ok(bytes)
}

pub(super) fn read_bundle(path: &Path) -> Result<ConcernAction> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Bundle {
        format_version: u32,
        observations: Vec<ConcernObservationInput>,
    }
    let bundle: Bundle = serde_json::from_slice(&read(path, 1024 * 1024)?)?;
    if bundle.format_version != 1 {
        bail!("unsupported concern recording format")
    }
    Ok(ConcernAction::Record {
        observations: bundle.observations,
    })
}
