// @feature code-concerns
// @feature runtime
// @spec docs/features/code-concerns.md
// @spec docs/features/runtime.md
// @boundary concern-reporting-json: short agent reports become the same typed immutable observations
use crate::code_concerns::*;
use anyhow::Result;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ObservationMetadata {
    id: Option<String>,
    occurred_at: Option<DateTime<Utc>>,
    observed_at: Option<DateTime<Utc>>,
    context: ConcernContext,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ReportingAction {
    Report {
        title: String,
        description: String,
        concern_id: Option<String>,
        hypothesis: Option<String>,
        impact: Option<String>,
        #[serde(default)]
        encounter: ConcernEncounter,
        #[serde(default)]
        observation: ObservationMetadata,
    },
    Upvote {
        concern_id: String,
        #[serde(default)]
        encounter: ConcernEncounter,
        fix_relation: Option<ConcernFixRelation>,
        #[serde(default)]
        observation: ObservationMetadata,
    },
}

impl ReportingAction {
    fn observation(self) -> ConcernObservationInput {
        let (metadata, concern_id, body) = match self {
            Self::Report {
                title,
                description,
                concern_id,
                hypothesis,
                impact,
                encounter,
                observation,
            } => (
                observation,
                concern_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                ConcernEvent::Reported {
                    title,
                    description,
                    hypothesis,
                    impact,
                    encounter,
                },
            ),
            Self::Upvote {
                concern_id,
                encounter,
                fix_relation,
                observation,
            } => (
                observation,
                concern_id,
                ConcernEvent::Encountered {
                    encounter,
                    fix_relation,
                },
            ),
        };
        ConcernObservationInput {
            id: metadata
                .id
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            concern_id,
            occurred_at: metadata.occurred_at,
            observed_at: metadata.observed_at.unwrap_or_else(Utc::now),
            context: metadata.context,
            body,
        }
    }
}

pub(super) fn prepare(arguments: Value) -> Result<ConcernAction> {
    if let Ok(action) = serde_json::from_value::<ConcernAction>(arguments.clone()) {
        return Ok(action);
    }
    let observation = serde_json::from_value::<ReportingAction>(arguments)?.observation();
    observation.validate()?;
    Ok(ConcernAction::Record {
        observations: vec![observation],
    })
}

pub(super) fn schema() -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(ConcernAction))
        .expect("concern schema serializable");
    let reporting = serde_json::to_value(schemars::schema_for!(ReportingAction))
        .expect("reporting schema serializable");
    schema["oneOf"]
        .as_array_mut()
        .expect("tagged actions")
        .extend(
            reporting["oneOf"]
                .as_array()
                .expect("tagged reporting actions")
                .iter()
                .cloned(),
        );
    if let Some(definitions) = reporting["$defs"].as_object() {
        let shared = schema["$defs"]
            .as_object_mut()
            .expect("concern definitions");
        for (name, definition) in definitions {
            shared
                .entry(name.clone())
                .or_insert_with(|| definition.clone());
        }
    }
    schema
}
