// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
mod inventory;
mod queries;
mod rows;
mod taxonomy;
use crate::analytics::{identity, types::*};
use crate::coordination::domain::ToolHookInput;
use crate::persistence::{backend::DbBackend, schema::*, Store};
use anyhow::Result;
use chrono::Utc;
use diesel::prelude::*;
use rows::*;

pub(super) fn scoped_key(kind: ResourceKind, name: &str, agent: &str, project: &str) -> String {
    let key = resource_key(kind, name, agent);
    if matches!(kind, ResourceKind::Script | ResourceKind::Inline) {
        format!("{project}:{key}")
    } else {
        key
    }
}

impl Store {
    pub(crate) fn record_activity_call(
        &mut self,
        project: &str,
        input: &ToolHookInput,
        actions: &[Action],
        outcome: &str,
    ) -> Result<()> {
        self.ensure_activity_categories()?;
        let call = AnalyticsCallsRow::from_hook(project, input, outcome);
        let insert = diesel::insert_into(analytics_calls::table)
            .values(&call)
            .on_conflict(analytics_calls::id);
        if outcome == "started" {
            insert.do_nothing().execute(&mut self.connection)?;
        } else {
            insert
                .do_update()
                .set((
                    analytics_calls::outcome.eq(&call.outcome),
                    analytics_calls::completed_at.eq(&call.completed_at),
                ))
                .execute(&mut self.connection)?;
        }
        let native = Resource {
            name: input.tool_name.clone(),
            kind: ResourceKind::Tool,
            origin: Origin::HostTool,
            exposure: Exposure::Declared,
            description: "Observed host tool".into(),
            activities: vec![],
            version: None,
        };
        self.record_resource(
            project,
            &native.key(&input.context.agent),
            &native,
            &call.observed_at,
        )?;
        for (index, action) in actions.iter().take(MAX_ACTIONS).enumerate() {
            self.record_action(&call, index, action)?;
        }
        Ok(())
    }

    fn record_action(
        &mut self,
        call: &AnalyticsCallsRow,
        index: usize,
        action: &Action,
    ) -> Result<()> {
        let project = &call.project_id;
        let key = scoped_key(action.kind, &action.name, &call.agent, project);
        let id = identity(&[&call.id, &index.to_string()]);
        let cached = self.purpose_for_action(project, &key, action)?;
        let ClassifiedPurpose {
            category,
            description,
            evidence,
            model,
        } = cached.unwrap_or_else(|| action.into());
        let category = self.canonical_category(&category)?;
        let resource_version = self.action_resource_version(project, &key, action)?;
        let row = AnalyticsActionsRow {
            id,
            call_id: call.id.clone(),
            project_id: project.into(),
            session_key: call.session_key.clone(),
            resource_key: key.clone(),
            name: action.name.clone(),
            kind: action.kind.as_str().into(),
            origin: action.origin.as_str().into(),
            category_id: category,
            description,
            pattern_key: action.pattern_key.clone(),
            resource_version,
            evidence,
            model_id: model,
            observed_at: call.observed_at.clone(),
        };
        self.record_resource(
            project,
            &key,
            &action.resource(&row.category_id, &row.description),
            &call.observed_at,
        )?;
        diesel::insert_into(analytics_actions::table)
            .values(&row)
            .on_conflict(analytics_actions::id)
            .do_nothing()
            .execute(&mut self.connection)?;
        Ok(())
    }

    fn action_resource_version(
        &mut self,
        project: &str,
        key: &str,
        action: &Action,
    ) -> Result<Option<String>> {
        if action.kind != ResourceKind::Tool {
            return Ok(Some(action.pattern_key.clone()));
        }
        Ok(analytics_resources::table
            .find(identity(&[project, key]))
            .select(analytics_resources::version)
            .first::<Option<String>>(&mut self.connection)
            .optional()?
            .flatten())
    }

    fn record_resource(
        &mut self,
        project: &str,
        key: &str,
        resource: &Resource,
        now: &str,
    ) -> Result<()> {
        let row = AnalyticsResourcesRow {
            id: identity(&[project, key]),
            project_id: project.into(),
            resource_key: key.into(),
            name: resource.name.clone(),
            kind: resource.kind.as_str().into(),
            origin: resource.origin.as_str().into(),
            description: resource.description.clone(),
            activities_json: serde_json::to_string(&resource.activities)?,
            version: resource.version.clone(),
            first_seen: now.into(),
            last_seen: now.into(),
        };
        diesel::insert_into(analytics_resources::table)
            .values(&row)
            .on_conflict(analytics_resources::id)
            .do_update()
            .set(analytics_resources::last_seen.eq(now))
            .execute(&mut self.connection)?;
        Ok(())
    }

    pub(crate) fn script_origin(
        &mut self,
        project: &str,
        session: &str,
        name: &str,
    ) -> Result<Origin> {
        let baseline = analytics_exposures::table
            .find(identity(&[session, "repository", "inventory-marker"]))
            .first::<AnalyticsExposuresRow>(&mut self.connection)
            .optional()?;
        let Some(baseline) = baseline else {
            return Ok(Origin::Unknown);
        };
        let key = scoped_key(ResourceKind::Script, name, "", project);
        let supplied = analytics_exposures::table
            .filter(analytics_exposures::project_id.eq(project))
            .filter(analytics_exposures::session_key.eq(session))
            .filter(analytics_exposures::resource_key.eq(&key))
            .filter(analytics_exposures::first_seen.le(&baseline.first_seen))
            .count()
            .get_result::<i64>(&mut self.connection)?;
        if supplied > 0 {
            return Ok(Origin::Repository);
        }
        let covered = ["scripts/", "tools/", ".githooks/"]
            .iter()
            .any(|prefix| name.starts_with(prefix));
        Ok(if covered && baseline.description == "complete" {
            Origin::AgentAuthored
        } else {
            Origin::Unknown
        })
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests;
