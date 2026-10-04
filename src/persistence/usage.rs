// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
// @entrypoint Store::usage_summary
use super::models::NewCapabilityUse;
use super::schema::capability_uses;
use super::Store;
use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use diesel::dsl::count_star;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;
use uuid::Uuid;

macro_rules! stored_enum {
    ($type:ty, $kind:literal, {$($variant:ident => $value:literal),+ $(,)?}) => {
        impl std::fmt::Display for $type {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(match self {
                    $(Self::$variant => $value),+
                })
            }
        }

        impl std::str::FromStr for $type {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    other => Err(format!("unknown {} {other}", $kind)),
                }
            }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Tool,
    Skill,
    Script,
}

stored_enum!(CapabilityKind, "capability kind", {
    Tool => "tool",
    Skill => "skill",
    Script => "script",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySource {
    HostHook,
    PiEvent,
    ShellInference,
    NexusExec,
    NexusInternal,
}

stored_enum!(CapabilitySource, "capability source", {
    HostHook => "host_hook",
    PiEvent => "pi_event",
    ShellInference => "shell_inference",
    NexusExec => "nexus_exec",
    NexusInternal => "nexus_internal",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEvidence {
    NativeHook,
    ExplicitInvocation,
    InstructionRead,
    AssetExecution,
    ShellParsed,
    Wrapped,
}

stored_enum!(CapabilityEvidence, "capability evidence", {
    NativeHook => "native_hook",
    ExplicitInvocation => "explicit_invocation",
    InstructionRead => "instruction_read",
    AssetExecution => "asset_execution",
    ShellParsed => "shell_parsed",
    Wrapped => "wrapped",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityOutcome {
    Pending,
    Succeeded,
    Failed,
    Interrupted,
    CompletedUnknown,
    Observed,
}

stored_enum!(CapabilityOutcome, "capability outcome", {
    Pending => "pending",
    Succeeded => "succeeded",
    Failed => "failed",
    Interrupted => "interrupted",
    CompletedUnknown => "completed_unknown",
    Observed => "observed",
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityObservation {
    pub project_id: String,
    pub agent: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub invocation_id: String,
    pub parent_invocation_id: Option<String>,
    pub kind: CapabilityKind,
    pub name: String,
    pub source: CapabilitySource,
    pub evidence: CapabilityEvidence,
    pub config_hash: String,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageCount {
    pub kind: CapabilityKind,
    pub name: String,
    pub count: i64,
    pub sessions: i64,
    pub pending: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub interrupted: i64,
    pub completed_unknown: i64,
    pub observed: i64,
    pub evidence: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageSummary {
    pub window_started_at: DateTime<Utc>,
    pub window_ended_at: DateTime<Utc>,
    pub tools: Vec<UsageCount>,
    pub skills: Vec<UsageCount>,
}

struct CapabilityWrite<'a> {
    observation: &'a CapabilityObservation,
    id: String,
    observed_at: String,
    kind: String,
    source: String,
    evidence: String,
    outcome: CapabilityOutcome,
    outcome_text: String,
}

impl<'a> CapabilityWrite<'a> {
    fn new(
        observation: &'a CapabilityObservation,
        outcome: CapabilityOutcome,
        observed_at: DateTime<Utc>,
    ) -> Self {
        Self {
            observation,
            id: Uuid::new_v4().to_string(),
            observed_at: observed_at.to_rfc3339(),
            kind: observation.kind.to_string(),
            source: observation.source.to_string(),
            evidence: observation.evidence.to_string(),
            outcome,
            outcome_text: outcome.to_string(),
        }
    }

    fn completed_at(&self) -> Option<&str> {
        (self.outcome != CapabilityOutcome::Pending).then_some(self.observed_at.as_str())
    }

    fn skill_turn_id(&self) -> Option<&str> {
        if self.observation.kind != CapabilityKind::Skill {
            return None;
        }
        self.observation.turn_id.as_deref()
    }

    fn new_row(&self) -> NewCapabilityUse<'_> {
        NewCapabilityUse {
            id: &self.id,
            project_id: &self.observation.project_id,
            agent: &self.observation.agent,
            session_id: &self.observation.session_id,
            turn_id: self.observation.turn_id.as_deref(),
            invocation_id: &self.observation.invocation_id,
            parent_invocation_id: self.observation.parent_invocation_id.as_deref(),
            kind: &self.kind,
            name: &self.observation.name,
            source: &self.source,
            evidence: &self.evidence,
            outcome: &self.outcome_text,
            config_hash: &self.observation.config_hash,
            model_id: self.observation.model_id.as_deref(),
            first_observed_at: &self.observed_at,
            completed_at: self.completed_at(),
        }
    }
}

struct UsageWindow<'a> {
    project_id: Option<&'a str>,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    start: String,
    end: String,
}

impl<'a> UsageWindow<'a> {
    fn new(
        project_id: Option<&'a str>,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
    ) -> Result<Self> {
        if ended_at <= started_at {
            bail!("usage window end must be after its start");
        }
        Ok(Self {
            project_id,
            start: started_at.to_rfc3339(),
            end: ended_at.to_rfc3339(),
            started_at,
            ended_at,
        })
    }
}

#[derive(Default)]
struct UsageAccumulator {
    counts: BTreeMap<(CapabilityKind, String), UsageCount>,
}

impl Store {
    pub fn observe_capability_start(&mut self, observation: &CapabilityObservation) -> Result<()> {
        self.observe_capability_at(observation, CapabilityOutcome::Pending, Utc::now())
    }

    pub fn observe_capability_completion(
        &mut self,
        observation: &CapabilityObservation,
        outcome: CapabilityOutcome,
    ) -> Result<()> {
        if outcome == CapabilityOutcome::Pending {
            bail!("capability completion cannot have a pending outcome");
        }
        self.observe_capability_at(observation, outcome, Utc::now())
    }

    fn observe_capability_at(
        &mut self,
        observation: &CapabilityObservation,
        outcome: CapabilityOutcome,
        observed_at: DateTime<Utc>,
    ) -> Result<()> {
        validate_observation(observation)?;
        let write = CapabilityWrite::new(observation, outcome, observed_at);
        self.connection
            .transaction::<_, anyhow::Error, _>(|connection| {
                insert_capability(connection, &write)?;
                promote_skill_evidence(connection, &write)?;
                update_capability_completion(connection, &write)?;
                Ok(())
            })
    }

    pub fn usage_summary(
        &mut self,
        project_id: Option<&str>,
        window_started_at: DateTime<Utc>,
        window_ended_at: DateTime<Utc>,
    ) -> Result<UsageSummary> {
        let window = UsageWindow::new(project_id, window_started_at, window_ended_at)?;
        let mut usage = UsageAccumulator::default();
        usage.add_dimensions(dimension_counts(&mut self.connection, &window)?)?;
        usage.add_sessions(session_counts(&mut self.connection, &window)?)?;
        Ok(usage.into_summary(window))
    }
}

fn insert_capability(
    connection: &mut SqliteConnection,
    write: &CapabilityWrite<'_>,
) -> QueryResult<()> {
    diesel::insert_into(capability_uses::table)
        .values(write.new_row())
        .on_conflict_do_nothing()
        .execute(connection)
        .map(|_| ())
}

fn promote_skill_evidence(
    connection: &mut SqliteConnection,
    write: &CapabilityWrite<'_>,
) -> QueryResult<()> {
    let Some(turn_id) = write.skill_turn_id() else {
        return Ok(());
    };
    let weaker = weaker_evidence(write.observation.evidence);
    if weaker.is_empty() {
        return Ok(());
    }
    diesel::update(
        capability_uses::table
            .filter(capability_uses::project_id.eq(&write.observation.project_id))
            .filter(capability_uses::agent.eq(&write.observation.agent))
            .filter(capability_uses::session_id.eq(&write.observation.session_id))
            .filter(capability_uses::kind.eq("skill"))
            .filter(capability_uses::turn_id.eq(turn_id))
            .filter(capability_uses::name.eq(&write.observation.name))
            .filter(capability_uses::evidence.eq_any(weaker)),
    )
    .set((
        capability_uses::source.eq(&write.source),
        capability_uses::evidence.eq(&write.evidence),
    ))
    .execute(connection)
    .map(|_| ())
}

fn update_capability_completion(
    connection: &mut SqliteConnection,
    write: &CapabilityWrite<'_>,
) -> QueryResult<()> {
    if write.outcome == CapabilityOutcome::Pending {
        return Ok(());
    }
    match write.skill_turn_id() {
        Some(turn_id) => update_skill_completion(connection, write, turn_id),
        None => update_invocation_completion(connection, write),
    }
}

fn update_skill_completion(
    connection: &mut SqliteConnection,
    write: &CapabilityWrite<'_>,
    turn_id: &str,
) -> QueryResult<()> {
    diesel::update(
        capability_uses::table
            .filter(capability_uses::project_id.eq(&write.observation.project_id))
            .filter(capability_uses::agent.eq(&write.observation.agent))
            .filter(capability_uses::session_id.eq(&write.observation.session_id))
            .filter(capability_uses::kind.eq(&write.kind))
            .filter(capability_uses::turn_id.eq(turn_id))
            .filter(capability_uses::name.eq(&write.observation.name))
            .filter(capability_uses::outcome.eq("pending")),
    )
    .set((
        capability_uses::outcome.eq(&write.outcome_text),
        capability_uses::completed_at.eq(Some(&write.observed_at)),
    ))
    .execute(connection)
    .map(|_| ())
}

fn update_invocation_completion(
    connection: &mut SqliteConnection,
    write: &CapabilityWrite<'_>,
) -> QueryResult<()> {
    diesel::update(
        capability_uses::table
            .filter(capability_uses::project_id.eq(&write.observation.project_id))
            .filter(capability_uses::agent.eq(&write.observation.agent))
            .filter(capability_uses::session_id.eq(&write.observation.session_id))
            .filter(capability_uses::kind.eq(&write.kind))
            .filter(capability_uses::invocation_id.eq(&write.observation.invocation_id))
            .filter(capability_uses::outcome.eq("pending")),
    )
    .set((
        capability_uses::outcome.eq(&write.outcome_text),
        capability_uses::completed_at.eq(Some(&write.observed_at)),
    ))
    .execute(connection)
    .map(|_| ())
}

impl UsageAccumulator {
    fn add_dimensions(&mut self, rows: Vec<DimensionCountRow>) -> Result<()> {
        rows.into_iter().try_for_each(|row| self.add_dimension(row))
    }

    fn add_dimension(&mut self, row: DimensionCountRow) -> Result<()> {
        let (kind, name, outcome, evidence, count) = row;
        let kind = CapabilityKind::from_str(&kind).map_err(anyhow::Error::msg)?;
        let outcome = CapabilityOutcome::from_str(&outcome).map_err(anyhow::Error::msg)?;
        self.counts
            .entry((kind, name.clone()))
            .or_insert_with(|| UsageCount::empty(kind, name))
            .record(outcome, evidence, count);
        Ok(())
    }

    fn add_sessions(&mut self, rows: Vec<SessionCountRow>) -> Result<()> {
        rows.into_iter().try_for_each(|row| self.add_session(row))
    }

    fn add_session(&mut self, row: SessionCountRow) -> Result<()> {
        let (kind, name, _, _, _) = row;
        let kind = CapabilityKind::from_str(&kind).map_err(anyhow::Error::msg)?;
        if let Some(count) = self.counts.get_mut(&(kind, name)) {
            count.sessions += 1;
        }
        Ok(())
    }

    fn into_summary(self, window: UsageWindow<'_>) -> UsageSummary {
        let (mut skills, mut tools): (Vec<_>, Vec<_>) = self
            .counts
            .into_values()
            .partition(|count| count.kind == CapabilityKind::Skill);
        sort_counts(&mut tools);
        sort_counts(&mut skills);
        UsageSummary {
            window_started_at: window.started_at,
            window_ended_at: window.ended_at,
            tools,
            skills,
        }
    }
}

impl UsageCount {
    fn empty(kind: CapabilityKind, name: String) -> Self {
        Self {
            kind,
            name,
            count: 0,
            sessions: 0,
            pending: 0,
            succeeded: 0,
            failed: 0,
            interrupted: 0,
            completed_unknown: 0,
            observed: 0,
            evidence: BTreeMap::new(),
        }
    }

    fn record(&mut self, outcome: CapabilityOutcome, evidence: String, count: i64) {
        self.count += count;
        match outcome {
            CapabilityOutcome::Pending => self.pending += count,
            CapabilityOutcome::Succeeded => self.succeeded += count,
            CapabilityOutcome::Failed => self.failed += count,
            CapabilityOutcome::Interrupted => self.interrupted += count,
            CapabilityOutcome::CompletedUnknown => self.completed_unknown += count,
            CapabilityOutcome::Observed => self.observed += count,
        }
        *self.evidence.entry(evidence).or_default() += count;
    }
}

fn validate_observation(observation: &CapabilityObservation) -> Result<()> {
    for (field, value) in [
        ("project_id", observation.project_id.as_str()),
        ("agent", observation.agent.as_str()),
        ("session_id", observation.session_id.as_str()),
        ("invocation_id", observation.invocation_id.as_str()),
        ("name", observation.name.as_str()),
        ("config_hash", observation.config_hash.as_str()),
    ] {
        if value.is_empty() {
            bail!("capability {field} cannot be empty");
        }
    }
    if observation.name.chars().count() > 512 {
        bail!("capability name exceeds 512 characters");
    }
    for (field, value) in [
        ("turn_id", observation.turn_id.as_deref()),
        (
            "parent_invocation_id",
            observation.parent_invocation_id.as_deref(),
        ),
        ("model_id", observation.model_id.as_deref()),
    ] {
        if value.is_some_and(str::is_empty) {
            bail!("capability {field} cannot be empty when present");
        }
    }
    Ok(())
}

type DimensionCountRow = (String, String, String, String, i64);
type SessionCountRow = (String, String, String, String, String);

fn dimension_counts(
    connection: &mut SqliteConnection,
    window: &UsageWindow<'_>,
) -> QueryResult<Vec<DimensionCountRow>> {
    let base = capability_uses::table
        .filter(capability_uses::first_observed_at.ge(&window.start))
        .filter(capability_uses::first_observed_at.lt(&window.end));
    if let Some(project_id) = window.project_id {
        base.filter(capability_uses::project_id.eq(project_id))
            .group_by((
                capability_uses::kind,
                capability_uses::name,
                capability_uses::outcome,
                capability_uses::evidence,
            ))
            .select((
                capability_uses::kind,
                capability_uses::name,
                capability_uses::outcome,
                capability_uses::evidence,
                count_star(),
            ))
            .load(connection)
    } else {
        base.group_by((
            capability_uses::kind,
            capability_uses::name,
            capability_uses::outcome,
            capability_uses::evidence,
        ))
        .select((
            capability_uses::kind,
            capability_uses::name,
            capability_uses::outcome,
            capability_uses::evidence,
            count_star(),
        ))
        .load(connection)
    }
}

fn session_counts(
    connection: &mut SqliteConnection,
    window: &UsageWindow<'_>,
) -> QueryResult<Vec<SessionCountRow>> {
    let base = capability_uses::table
        .filter(capability_uses::first_observed_at.ge(&window.start))
        .filter(capability_uses::first_observed_at.lt(&window.end));
    if let Some(project_id) = window.project_id {
        base.filter(capability_uses::project_id.eq(project_id))
            .select((
                capability_uses::kind,
                capability_uses::name,
                capability_uses::project_id,
                capability_uses::agent,
                capability_uses::session_id,
            ))
            .distinct()
            .load(connection)
    } else {
        base.select((
            capability_uses::kind,
            capability_uses::name,
            capability_uses::project_id,
            capability_uses::agent,
            capability_uses::session_id,
        ))
        .distinct()
        .load(connection)
    }
}

fn weaker_evidence(evidence: CapabilityEvidence) -> Vec<&'static str> {
    const ORDER: &[(CapabilityEvidence, &str)] = &[
        (CapabilityEvidence::ShellParsed, "shell_parsed"),
        (CapabilityEvidence::Wrapped, "wrapped"),
        (CapabilityEvidence::AssetExecution, "asset_execution"),
        (CapabilityEvidence::InstructionRead, "instruction_read"),
        (
            CapabilityEvidence::ExplicitInvocation,
            "explicit_invocation",
        ),
        (CapabilityEvidence::NativeHook, "native_hook"),
    ];
    let position = ORDER
        .iter()
        .position(|(candidate, _)| *candidate == evidence)
        .unwrap_or(0);
    ORDER[..position].iter().map(|(_, value)| *value).collect()
}

fn sort_counts(counts: &mut [UsageCount]) {
    counts.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.kind.cmp(&right.kind))
    });
}

#[cfg(test)]
mod tests;
