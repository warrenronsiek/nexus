// @feature persistence
// @feature work-history
// @feature code-concerns
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// @spec docs/features/code-concerns.md
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The first receiving project's receipt, retained across later imports.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordSource<Actor> {
    pub project_id: String,
    pub recorded_at: DateTime<Utc>,
    pub provenance: Actor,
}

impl<Actor: Clone> RecordSource<Actor> {
    pub(crate) fn retained(
        source: Option<&Self>,
        project_id: &str,
        recorded_at: DateTime<Utc>,
        provenance: &Actor,
    ) -> Self {
        source.cloned().unwrap_or_else(|| Self {
            project_id: project_id.into(),
            recorded_at,
            provenance: provenance.clone(),
        })
    }
}
