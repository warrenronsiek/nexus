// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
use crate::persistence::schema::*;
use diesel::prelude::*;

#[derive(Clone, Debug, Insertable, Queryable, Selectable)]
#[diesel(table_name=analytics_calls)]
pub(super) struct AnalyticsCallsRow {
    pub id: String,
    pub project_id: String,
    pub agent: String,
    pub session_key: String,
    pub invocation_id: String,
    pub parent_id: Option<String>,
    pub tool_name: String,
    pub outcome: String,
    pub observed_at: String,
    pub completed_at: Option<String>,
}

#[derive(Clone, Debug, Insertable, Queryable, Selectable)]
#[diesel(table_name=analytics_actions)]
pub(super) struct AnalyticsActionsRow {
    pub id: String,
    pub call_id: String,
    pub project_id: String,
    pub session_key: String,
    pub resource_key: String,
    pub name: String,
    pub kind: String,
    pub origin: String,
    pub category_id: String,
    pub description: String,
    pub pattern_key: String,
    pub resource_version: Option<String>,
    pub evidence: String,
    pub model_id: Option<String>,
    pub observed_at: String,
}

#[derive(Clone, Debug, Insertable, Queryable, Selectable)]
#[diesel(table_name=analytics_exposures)]
pub(super) struct AnalyticsExposuresRow {
    pub id: String,
    pub project_id: String,
    pub session_key: String,
    pub scope: String,
    pub resource_key: String,
    pub name: String,
    pub kind: String,
    pub origin: String,
    pub exposure: String,
    pub version: Option<String>,
    pub description: String,
    pub activities_json: String,
    pub first_seen: String,
    pub last_seen: String,
    pub active: bool,
}

#[derive(Clone, Debug, Insertable, Queryable, Selectable)]
#[diesel(table_name=analytics_categories)]
pub(super) struct AnalyticsCategoriesRow {
    pub id: String,
    pub name: String,
    pub definition: String,
    pub replaced_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Insertable, Queryable, Selectable)]
#[diesel(table_name=analytics_revisions)]
pub(super) struct AnalyticsRevisionsRow {
    pub id: String,
    pub action_id: String,
    pub category_id: String,
    pub description: String,
    pub evidence: String,
    pub model_id: Option<String>,
    pub policy_version: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Insertable, Queryable, Selectable)]
#[diesel(table_name=analytics_resources)]
pub(super) struct AnalyticsResourcesRow {
    pub id: String,
    pub project_id: String,
    pub resource_key: String,
    pub name: String,
    pub kind: String,
    pub origin: String,
    pub description: String,
    pub activities_json: String,
    pub version: Option<String>,
    pub first_seen: String,
    pub last_seen: String,
}

impl AnalyticsCallsRow {
    pub(super) fn from_hook(
        project: &str,
        input: &crate::coordination::domain::ToolHookInput,
        outcome: &str,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        let session =
            crate::analytics::identity(&[project, &input.context.agent, &input.context.session_id]);
        let call_id = crate::analytics::identity(&[&session, &input.tool_use_id]);
        Self {
            id: call_id.clone(),
            project_id: project.into(),
            agent: input.context.agent.clone(),
            session_key: session.clone(),
            invocation_id: input.tool_use_id.clone(),
            parent_id: input
                .parent_tool_use_id
                .as_deref()
                .map(|id| crate::analytics::identity(&[&session, id])),
            tool_name: input.tool_name.clone(),
            outcome: outcome.into(),
            observed_at: now.clone(),
            completed_at: (outcome != "started").then_some(now),
        }
    }
}

#[derive(Queryable)]
pub(super) struct ClassifiedPurpose {
    pub category: String,
    pub description: String,
    pub evidence: String,
    pub model: Option<String>,
}
impl From<&crate::analytics::types::Action> for ClassifiedPurpose {
    fn from(action: &crate::analytics::types::Action) -> Self {
        Self {
            category: action.category_id.clone(),
            description: action.description.clone(),
            evidence: if action.category_id == "unclassified" {
                "pending"
            } else {
                "deterministic"
            }
            .into(),
            model: None,
        }
    }
}
