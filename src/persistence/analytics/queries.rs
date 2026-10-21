// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
use super::*;
use diesel::dsl::{count, count_star};

macro_rules! leaf_actions {
    () => {
        analytics_actions::table.filter(
            analytics_actions::call_id.ne_all(
                analytics_calls::table
                    .filter(analytics_calls::parent_id.is_not_null())
                    .select(analytics_calls::parent_id.assume_not_null()),
            ),
        )
    };
}

fn window_start() -> String {
    (Utc::now() - chrono::Duration::days(7)).to_rfc3339()
}
pub(super) fn category_name(id: &str) -> String {
    id.replace(['.', '_', '-'], " ")
}

impl Store {
    pub(crate) fn activity_examples(
        &mut self,
        project: Option<&str>,
        category: Option<&str>,
        resource: Option<&str>,
        offset: u32,
    ) -> Result<ExamplesPage> {
        let mut query = analytics_actions::table
            .inner_join(
                analytics_calls::table.on(analytics_actions::call_id.eq(analytics_calls::id)),
            )
            .filter(analytics_actions::observed_at.ge(window_start()))
            .select((
                analytics_actions::name,
                analytics_actions::description,
                analytics_actions::origin,
                analytics_actions::evidence,
                analytics_calls::tool_name,
                analytics_calls::outcome,
                analytics_actions::model_id,
                analytics_actions::observed_at,
                analytics_actions::category_id,
            ))
            .order((
                analytics_actions::observed_at.desc(),
                analytics_actions::id.asc(),
            ))
            .limit(33)
            .offset(i64::from(offset))
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            query = query.filter(analytics_actions::project_id.eq(project));
        }
        if let Some(category) = category {
            query = query.filter(analytics_actions::category_id.eq(category));
        }
        if let Some(resource) = resource {
            if let Some((agent, name)) = native_tool_key(resource) {
                query = query
                    .filter(analytics_calls::agent.eq(agent))
                    .filter(analytics_calls::tool_name.eq(name));
            } else {
                query = query.filter(analytics_actions::resource_key.eq(resource));
            }
        }
        let mut examples = query.load::<ActivityExample>(&mut self.connection)?;
        let next_offset = (examples.len() > 32).then_some(offset.saturating_add(32));
        examples.truncate(32);
        Ok(ExamplesPage {
            ok: true,
            examples,
            next_offset,
        })
    }

    pub(crate) fn activity_summary(&mut self, project: Option<&str>) -> Result<ActivitySummary> {
        let start = window_start();
        let end = Utc::now().to_rfc3339();
        let (observed_calls, identified_actions, unclassified_actions) =
            self.activity_totals(project, &start, &end)?;
        let rows = self.activity_dimensions(project, &start, &end)?;
        let truncated = rows.len() > 64;
        let improvised = self.improvised_counts(project)?;
        let ids: Vec<&str> = rows.iter().map(|row| row.0.as_str()).collect();
        let names = analytics_categories::table
            .filter(analytics_categories::id.eq_any(ids))
            .select((analytics_categories::id, analytics_categories::name))
            .load::<(String, String)>(&mut self.connection)?
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();
        let activities = rows
            .into_iter()
            .take(64)
            .map(|(category, calls, actions, sessions)| ActivityCount {
                name: names
                    .get(&category)
                    .cloned()
                    .unwrap_or_else(|| category_name(&category)),
                improvised_actions: improvised.get(&category).map_or(0, |row| row.1),
                category_id: category,
                calls,
                actions,
                sessions,
            })
            .collect();
        Ok(ActivitySummary {ok:true,observed_calls,identified_actions,classified_actions:identified_actions-unclassified_actions,unclassified_actions,activities,truncated,window_started_at:start,window_ended_at:end,
            counting:"Calls are actual tool invocations; actions are identified operations. Category call counts overlap for multi-activity calls; no adoption percentage is implied."})
    }

    fn activity_totals(
        &mut self,
        project: Option<&str>,
        start: &str,
        end: &str,
    ) -> Result<(i64, i64, i64)> {
        let mut calls = analytics_calls::table
            .filter(analytics_calls::observed_at.ge(start))
            .filter(analytics_calls::observed_at.lt(end))
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            calls = calls.filter(analytics_calls::project_id.eq(project));
        }
        let observed_calls = calls.count().get_result(&mut self.connection)?;
        let mut actions = leaf_actions!()
            .filter(analytics_actions::observed_at.ge(start))
            .filter(analytics_actions::observed_at.lt(end))
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            actions = actions.filter(analytics_actions::project_id.eq(project));
        }
        let identified_actions = actions.count().get_result::<i64>(&mut self.connection)?;
        Ok((
            observed_calls,
            identified_actions,
            self.unclassified_count(project, start, end)?,
        ))
    }
    fn activity_dimensions(
        &mut self,
        project: Option<&str>,
        start: &str,
        end: &str,
    ) -> Result<Vec<(String, i64, i64, i64)>> {
        let mut grouped = leaf_actions!()
            .filter(analytics_actions::observed_at.ge(start))
            .filter(analytics_actions::observed_at.lt(end))
            .group_by(analytics_actions::category_id)
            .select((
                analytics_actions::category_id,
                count(analytics_actions::call_id).aggregate_distinct(),
                count_star(),
                count(analytics_actions::session_key).aggregate_distinct(),
            ))
            .order((count_star().desc(), analytics_actions::category_id.asc()))
            .limit(65)
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            grouped = grouped.filter(analytics_actions::project_id.eq(project));
        }
        let rows = grouped.load::<(String, i64, i64, i64)>(&mut self.connection)?;
        Ok(rows)
    }

    fn unclassified_count(&mut self, project: Option<&str>, start: &str, end: &str) -> Result<i64> {
        let mut query = leaf_actions!()
            .filter(analytics_actions::observed_at.ge(start))
            .filter(analytics_actions::observed_at.lt(end))
            .filter(analytics_actions::category_id.eq("unclassified"))
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            query = query.filter(analytics_actions::project_id.eq(project));
        }
        Ok(query.count().get_result(&mut self.connection)?)
    }

    fn improvised_counts(
        &mut self,
        project: Option<&str>,
    ) -> Result<std::collections::HashMap<String, (i64, i64, i64)>> {
        let mut query = leaf_actions!()
            .filter(analytics_actions::observed_at.ge(window_start()))
            .filter(analytics_actions::origin.eq_any(["agent_authored", "standard_cli"]))
            .group_by(analytics_actions::category_id)
            .select((
                analytics_actions::category_id,
                count(analytics_actions::call_id).aggregate_distinct(),
                count_star(),
                count(analytics_actions::session_key).aggregate_distinct(),
            ))
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            query = query.filter(analytics_actions::project_id.eq(project));
        }
        Ok(query
            .load::<(String, i64, i64, i64)>(&mut self.connection)?
            .into_iter()
            .map(|(id, calls, actions, sessions)| (id, (calls, actions, sessions)))
            .collect())
    }

    pub(crate) fn activity_catalog(
        &mut self,
        project: Option<&str>,
        offset: u32,
        limit: u32,
    ) -> Result<CatalogPage> {
        let limit = limit.clamp(1, 64);
        let rows = self.catalog_dimensions(project, offset, limit)?;
        let next_offset = (rows.len() > limit as usize).then_some(offset.saturating_add(limit));
        let keys: Vec<&str> = rows.iter().map(|row| row.0.as_str()).collect();
        let calls = self.capability_calls(project, &keys)?;
        let exposure = self.exposure_counts(project, &keys)?;
        let capabilities = rows
            .into_iter()
            .take(limit as usize)
            .map(|(key, name, kind, origin, description)| {
                let offered_sessions =
                    *exposure.get(&(key.clone(), "exposed".into())).unwrap_or(&0);
                let discoverable_sessions = *exposure
                    .get(&(key.clone(), "discoverable".into()))
                    .unwrap_or(&0);
                let known = exposure.keys().any(|(resource, _)| resource == &key);
                CapabilityCount {
                    calls: *calls.get(&key).unwrap_or(&0),
                    resource_key: key,
                    name,
                    kind,
                    origin,
                    offered_sessions,
                    discoverable_sessions,
                    description: description.unwrap_or_default(),
                    availability: if known {
                        "observed_inventory"
                    } else {
                        "unknown"
                    },
                }
            })
            .collect();
        Ok(CatalogPage {
            ok: true,
            capabilities,
            next_offset,
        })
    }

    fn catalog_dimensions(
        &mut self,
        project: Option<&str>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<CatalogDimension>> {
        let mut query = analytics_resources::table
            .filter(analytics_resources::last_seen.ge(window_start()))
            .group_by((
                analytics_resources::resource_key,
                analytics_resources::name,
                analytics_resources::kind,
                analytics_resources::origin,
            ))
            .select((
                analytics_resources::resource_key,
                analytics_resources::name,
                analytics_resources::kind,
                analytics_resources::origin,
                diesel::dsl::max(analytics_resources::description),
            ))
            .order((
                analytics_resources::name.asc(),
                analytics_resources::resource_key.asc(),
            ))
            .limit(i64::from(limit) + 1)
            .offset(i64::from(offset))
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            query = query.filter(analytics_resources::project_id.eq(project));
        }
        let rows =
            query.load::<(String, String, String, String, Option<String>)>(&mut self.connection)?;
        Ok(rows)
    }

    fn capability_calls(
        &mut self,
        project: Option<&str>,
        keys: &[&str],
    ) -> Result<std::collections::HashMap<String, i64>> {
        let mut query = analytics_actions::table
            .filter(analytics_actions::observed_at.ge(window_start()))
            .filter(analytics_actions::resource_key.eq_any(keys))
            .filter(analytics_actions::kind.ne("tool"))
            .group_by(analytics_actions::resource_key)
            .select((
                analytics_actions::resource_key,
                count(analytics_actions::call_id).aggregate_distinct(),
            ))
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            query = query.filter(analytics_actions::project_id.eq(project));
        }
        let mut calls = query
            .load::<(String, i64)>(&mut self.connection)?
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();
        let mut native = analytics_calls::table
            .filter(analytics_calls::observed_at.ge(window_start()))
            .group_by((analytics_calls::agent, analytics_calls::tool_name))
            .select((
                analytics_calls::agent,
                analytics_calls::tool_name,
                count_star(),
            ))
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            native = native.filter(analytics_calls::project_id.eq(project));
        }
        for (agent, name, count) in native.load::<(String, String, i64)>(&mut self.connection)? {
            calls.insert(resource_key(ResourceKind::Tool, &name, &agent), count);
        }
        Ok(calls)
    }

    fn exposure_counts(
        &mut self,
        project: Option<&str>,
        keys: &[&str],
    ) -> Result<std::collections::HashMap<(String, String), i64>> {
        let mut query = analytics_exposures::table
            .filter(analytics_exposures::last_seen.ge(window_start()))
            .filter(analytics_exposures::resource_key.eq_any(keys))
            .group_by((
                analytics_exposures::resource_key,
                analytics_exposures::exposure,
            ))
            .select((
                analytics_exposures::resource_key,
                analytics_exposures::exposure,
                count(analytics_exposures::session_key).aggregate_distinct(),
            ))
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            query = query.filter(analytics_exposures::project_id.eq(project));
        }
        Ok(query
            .load::<(String, String, i64)>(&mut self.connection)?
            .into_iter()
            .map(|(key, phase, count)| ((key, phase), count))
            .collect())
    }

    pub(crate) fn activity_opportunities(
        &mut self,
        project: Option<&str>,
    ) -> Result<Opportunities> {
        let summary = self.activity_summary(project)?;
        let counts = self.improvised_counts(project)?;
        let (implementations, resources, truncated) = self.opportunity_evidence(project)?;
        let aliases = analytics_categories::table
            .select((analytics_categories::id, analytics_categories::replaced_by))
            .load::<(String, Option<String>)>(&mut self.connection)?
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();
        let mut opportunities = Vec::new();
        for item in summary
            .activities
            .iter()
            .filter(|item| item.improvised_actions > 0 && item.category_id != "unclassified")
        {
            let supplied_alternatives =
                offered_alternatives(&item.category_id, &implementations, &resources, &aliases)?;
            let (calls, _, sessions) = counts.get(&item.category_id).copied().unwrap_or_default();
            opportunities.push(Opportunity {category_id:item.category_id.clone(),name:item.name.clone(),calls,sessions,supplied_alternatives,evidence:"Same-category capabilities offered in matching sessions; suitability and availability at each call remain unknown."});
        }
        opportunities.sort_by(|a, b| {
            b.sessions
                .cmp(&a.sessions)
                .then(b.calls.cmp(&a.calls))
                .then(a.category_id.cmp(&b.category_id))
        });
        Ok(Opportunities {
            ok: true,
            opportunities,
            truncated: summary.truncated || truncated,
        })
    }

    fn opportunity_evidence(&mut self, project: Option<&str>) -> Result<OpportunityEvidence> {
        let mut actions = leaf_actions!()
            .filter(analytics_actions::observed_at.ge(window_start()))
            .filter(analytics_actions::origin.eq_any(["agent_authored", "standard_cli"]))
            .select((
                analytics_actions::category_id,
                analytics_actions::session_key,
            ))
            .distinct()
            .order((
                analytics_actions::category_id,
                analytics_actions::session_key,
            ))
            .limit(4097)
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            actions = actions.filter(analytics_actions::project_id.eq(project));
        }
        let mut implementations = actions.load::<(String, String)>(&mut self.connection)?;
        let mut resources = self.offered_resource_evidence(project)?;
        let truncated = implementations.len() > 4096 || resources.len() > 4096;
        implementations.truncate(4096);
        resources.truncate(4096);
        Ok((implementations, resources, truncated))
    }
    fn offered_resource_evidence(
        &mut self,
        project: Option<&str>,
    ) -> Result<Vec<(String, String, String)>> {
        let mut exposures = analytics_exposures::table
            .inner_join(
                analytics_resources::table.on(analytics_exposures::resource_key
                    .eq(analytics_resources::resource_key)
                    .and(analytics_exposures::project_id.eq(analytics_resources::project_id))),
            )
            .filter(analytics_exposures::last_seen.ge(window_start()))
            .filter(analytics_exposures::exposure.eq("exposed"))
            .filter(analytics_resources::origin.eq_any(["host_tool", "repository"]))
            .select((
                analytics_exposures::session_key,
                analytics_resources::name,
                analytics_resources::activities_json,
            ))
            .distinct()
            .order((
                analytics_exposures::session_key,
                analytics_resources::name,
                analytics_resources::activities_json,
            ))
            .limit(4097)
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            exposures = exposures.filter(analytics_exposures::project_id.eq(project));
        }
        let resources = exposures.load::<(String, String, String)>(&mut self.connection)?;
        Ok(resources)
    }
}
type OpportunityEvidence = (Vec<(String, String)>, Vec<(String, String, String)>, bool);
fn canonical_alias(
    id: &str,
    aliases: &std::collections::HashMap<String, Option<String>>,
) -> String {
    let mut id = id.to_owned();
    for _ in 0..16 {
        match aliases.get(&id).and_then(Option::as_ref) {
            Some(next) => id = next.clone(),
            None => break,
        }
    }
    id
}

type CatalogDimension = (String, String, String, String, Option<String>);

fn offered_alternatives(
    category: &str,
    implementations: &[(String, String)],
    resources: &[(String, String, String)],
    aliases: &std::collections::HashMap<String, Option<String>>,
) -> Result<Vec<String>> {
    let wanted = category;
    let sessions = implementations
        .iter()
        .filter(|(category, _)| category == wanted)
        .map(|(_, session)| session)
        .collect::<std::collections::HashSet<_>>();
    let mut supplied_alternatives = Vec::new();
    for (session, name, activities) in resources {
        let ids: Vec<String> = serde_json::from_str(activities)?;
        if sessions.contains(session)
            && ids
                .iter()
                .any(|id| canonical_alias(id, aliases) == category)
        {
            supplied_alternatives.push(name.clone());
        }
    }
    supplied_alternatives.sort();
    supplied_alternatives.dedup();
    Ok(supplied_alternatives)
}
