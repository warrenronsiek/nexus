// @feature agent-traces
// @feature persistence
// @spec docs/features/agent-traces.md
// @spec docs/features/persistence.md
// @boundary trace-rows: portable immutable observations carry JSON content that is detached from query summaries.
use super::{canonical_time, content_hash};
use crate::agent_traces::{
    TraceBatch, TraceEvent, TraceInvocation, TraceReceipt, MAX_BATCH_BYTES, MAX_BATCH_EVENTS,
};
use crate::persistence::models::parse_time;
use crate::persistence::schema::{trace_batches, trace_events, trace_invocations};
use anyhow::{ensure, Result};
use chrono::{SecondsFormat, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Insertable, Queryable, Selectable, Serialize)]
#[serde(deny_unknown_fields)]
#[diesel(table_name = trace_batches)]
pub(in crate::persistence) struct BatchRow {
    pub id: String,
    pub project_id: String,
    pub application_id: String,
    pub environment: String,
    pub batch_id: String,
    pub content_hash: String,
    pub received_at: String,
    pub accepted_events: i32,
    pub duplicate_events: i32,
    pub projected: bool,
}

impl BatchRow {
    pub(super) fn new(project: &str, batch: &TraceBatch) -> Result<Self> {
        let encoded = serde_json::to_vec(batch)?;
        ensure!(
            encoded.len() <= MAX_BATCH_BYTES,
            "trace batch exceeds its encoded byte limit"
        );
        Ok(Self {
            id: content_hash(&[
                project,
                &batch.application_id,
                &batch.environment,
                &batch.batch_id,
            ])?,
            project_id: project.to_owned(),
            application_id: batch.application_id.clone(),
            environment: batch.environment.clone(),
            batch_id: batch.batch_id.clone(),
            content_hash: blake3::hash(&encoded).to_hex().to_string(),
            received_at: Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true),
            accepted_events: 0,
            duplicate_events: 0,
            projected: false,
        })
    }

    pub(in crate::persistence) fn validate(&self) -> Result<()> {
        let total_events = i64::from(self.accepted_events) + i64::from(self.duplicate_events);
        ensure!(
            self.id
                == content_hash(&[
                    &self.project_id,
                    &self.application_id,
                    &self.environment,
                    &self.batch_id,
                ])?
                && canonical_time(&self.received_at)? == self.received_at
                && valid_hash(&self.content_hash)
                && self.accepted_events >= 0
                && self.duplicate_events >= 0
                && (1..=MAX_BATCH_EVENTS as i64).contains(&total_events),
            "invalid portable trace batch receipt"
        );
        Ok(())
    }

    pub(super) fn receipt(self) -> Result<TraceReceipt> {
        Ok(TraceReceipt {
            batch_id: self.batch_id,
            received_at: self.received_at,
            accepted_events: usize::try_from(self.accepted_events)?,
            duplicate_events: usize::try_from(self.duplicate_events)?,
        })
    }
}

#[derive(Deserialize, Insertable, Queryable, Selectable, Serialize)]
#[serde(deny_unknown_fields)]
#[diesel(table_name = trace_events, treat_none_as_default_value = false)]
pub(in crate::persistence) struct EventRow {
    pub id: String,
    pub batch_key: String,
    pub project_id: String,
    pub application_id: String,
    pub environment: String,
    pub trace_id: String,
    pub event_id: String,
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub occurred_at: String,
    pub source_sequence: Option<i64>,
    pub event_json: String,
    pub payload_json: Option<String>,
    pub payload_bytes: i64,
    pub payload_expired: bool,
    pub content_hash: String,
}

impl EventRow {
    pub(super) fn key(
        project: &str,
        application: &str,
        environment: &str,
        event: &str,
    ) -> Result<String> {
        content_hash(&[project, application, environment, event])
    }

    pub(super) fn new(batch: &BatchRow, mut event: TraceEvent) -> Result<Self> {
        let digest = content_hash(&event)?;
        let payload_json = event
            .payload
            .take()
            .map(|value| serde_json::to_string(&value))
            .transpose()?;
        let event_json = serde_json::to_string(&event)?;
        Ok(Self {
            id: Self::key(
                &batch.project_id,
                &batch.application_id,
                &batch.environment,
                &event.event_id,
            )?,
            batch_key: batch.id.clone(),
            project_id: batch.project_id.clone(),
            application_id: batch.application_id.clone(),
            environment: batch.environment.clone(),
            occurred_at: canonical_time(&event.occurred_at)?,
            trace_id: event.trace_id,
            event_id: event.event_id,
            agent_id: event.agent_id,
            conversation_id: event.conversation_id,
            source_sequence: event.sequence,
            event_json,
            payload_bytes: payload_json.as_ref().map_or(0, String::len) as i64,
            payload_json,
            payload_expired: false,
            content_hash: digest,
        })
    }

    pub(in crate::persistence) fn validate(&self) -> Result<()> {
        let mut event: TraceEvent = serde_json::from_str(&self.event_json)?;
        ensure!(
            event.payload.is_none(),
            "portable trace metadata contains a payload"
        );
        event.payload = self
            .payload_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?;
        event.validate()?;
        ensure!(
            self.validate_header(&event)? && self.validate_fingerprint(&event)?,
            "portable trace event does not match its immutable content"
        );
        Ok(())
    }
    fn validate_header(&self, event: &TraceEvent) -> Result<bool> {
        Ok(self.id
            == Self::key(
                &self.project_id,
                &self.application_id,
                &self.environment,
                &event.event_id,
            )?
            && self.event_id == event.event_id
            && self.trace_id == event.trace_id
            && self.agent_id == event.agent_id
            && self.conversation_id == event.conversation_id
            && self.payload_bytes == self.payload_json.as_ref().map_or(0, String::len) as i64
            && self.source_sequence == event.sequence
            && self.occurred_at == canonical_time(&event.occurred_at)?)
    }

    fn validate_fingerprint(&self, event: &TraceEvent) -> Result<bool> {
        if self.payload_expired {
            Ok(self.payload_json.is_none()
                && self.payload_bytes == 0
                && valid_hash(&self.content_hash))
        } else {
            Ok(self.content_hash == content_hash(event)?)
        }
    }
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Insertable, Queryable, Selectable)]
#[diesel(table_name = trace_invocations, treat_none_as_default_value = false)]
pub(super) struct InvocationRow {
    pub id: String,
    pub project_id: String,
    pub application_id: String,
    pub environment: String,
    pub trace_id: String,
    pub agent_id: String,
    pub invocation_id: String,
    pub kind: String,
    pub name: Option<String>,
    pub version: Option<String>,
    pub occurred_at: String,
    pub outcome: Option<String>,
    pub duration_ms: Option<i64>,
    pub started_event_id: Option<String>,
    pub finished_event_id: Option<String>,
}

impl InvocationRow {
    pub(super) fn decode(self) -> Result<TraceInvocation> {
        Ok(TraceInvocation {
            application_id: self.application_id,
            environment: self.environment,
            trace_id: self.trace_id,
            agent_id: self.agent_id,
            invocation_id: self.invocation_id,
            kind: self.kind.parse()?,
            name: self.name,
            version: self.version,
            occurred_at: parse_time(&self.occurred_at)?.into(),
            outcome: self.outcome.as_deref().map(str::parse).transpose()?,
            duration_ms: self.duration_ms,
            started_event_id: self.started_event_id,
            finished_event_id: self.finished_event_id,
        })
    }
}
