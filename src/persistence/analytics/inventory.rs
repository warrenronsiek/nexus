// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
use super::*;
use crate::coordination::domain::HookContext;

struct Snapshot<'a> {
    project: &'a str,
    agent: &'a str,
    session: String,
    scope: &'a str,
    now: String,
    complete: bool,
}
impl<'a> Snapshot<'a> {
    fn new(project: &'a str, context: &'a HookContext, scope: &'a str, complete: bool) -> Self {
        Self {
            project,
            agent: &context.agent,
            session: identity(&[project, &context.agent, &context.session_id]),
            scope,
            now: Utc::now().to_rfc3339(),
            complete,
        }
    }
    fn exposure(&self, resource: &Resource) -> Result<AnalyticsExposuresRow> {
        let key = scoped_key(resource.kind, &resource.name, self.agent, self.project);
        Ok(AnalyticsExposuresRow {
            id: identity(&[
                &self.session,
                self.scope,
                &key,
                resource.exposure.as_str(),
                resource.version.as_deref().unwrap_or(""),
            ]),
            project_id: self.project.into(),
            session_key: self.session.clone(),
            scope: self.scope.into(),
            resource_key: key,
            name: resource.name.clone(),
            kind: resource.kind.as_str().into(),
            origin: resource.origin.as_str().into(),
            exposure: resource.exposure.as_str().into(),
            version: resource.version.clone(),
            description: resource.description.clone(),
            activities_json: serde_json::to_string(&resource.activities)?,
            first_seen: self.now.clone(),
            last_seen: self.now.clone(),
            active: true,
        })
    }
    fn marker(&self) -> AnalyticsExposuresRow {
        AnalyticsExposuresRow {
            id: identity(&[&self.session, self.scope, "inventory-marker"]),
            project_id: self.project.into(),
            session_key: self.session.clone(),
            scope: self.scope.into(),
            resource_key: "inventory-marker".into(),
            name: "Inventory coverage".into(),
            kind: "inventory".into(),
            origin: "unknown".into(),
            exposure: "declared".into(),
            version: None,
            description: if self.complete { "complete" } else { "partial" }.into(),
            activities_json: "[]".into(),
            first_seen: self.now.clone(),
            last_seen: self.now.clone(),
            active: true,
        }
    }
}
impl Store {
    pub(crate) fn record_inventory(
        &mut self,
        project: &str,
        context: &HookContext,
        scope: &str,
        resources: &[Resource],
        complete: bool,
    ) -> Result<()> {
        crate::analytics::inventory::validate(scope, resources)?;
        let snapshot = Snapshot::new(project, context, scope, complete);
        let rows = resources
            .iter()
            .map(|resource| snapshot.exposure(resource))
            .collect::<Result<Vec<_>>>()?;
        self.write_exposures(&snapshot, &rows)?;
        for resource in resources {
            self.refresh_inventory_resource(&snapshot, resource)?;
        }
        Ok(())
    }
    fn write_exposures(
        &mut self,
        snapshot: &Snapshot<'_>,
        rows: &[AnalyticsExposuresRow],
    ) -> Result<()> {
        self.connection
            .transaction::<_, anyhow::Error, _>(|connection| {
                if snapshot.complete {
                    diesel::update(
                        analytics_exposures::table
                            .filter(analytics_exposures::project_id.eq(snapshot.project))
                            .filter(analytics_exposures::session_key.eq(&snapshot.session))
                            .filter(analytics_exposures::scope.eq(snapshot.scope)),
                    )
                    .set(analytics_exposures::active.eq(false))
                    .execute(connection)?;
                }
                diesel::insert_into(analytics_exposures::table)
                    .values(snapshot.marker())
                    .on_conflict(analytics_exposures::id)
                    .do_nothing()
                    .execute(connection)?;
                for row in rows {
                    write_exposure(connection, row)?;
                }
                Ok(())
            })
    }
    fn refresh_inventory_resource(
        &mut self,
        snapshot: &Snapshot<'_>,
        resource: &Resource,
    ) -> Result<()> {
        let key = scoped_key(
            resource.kind,
            &resource.name,
            snapshot.agent,
            snapshot.project,
        );
        self.record_resource(snapshot.project, &key, resource, &snapshot.now)?;
        let previous = analytics_resources::table
            .find(identity(&[snapshot.project, &key]))
            .first::<AnalyticsResourcesRow>(&mut self.connection)?;
        let changed =
            previous.version != resource.version || previous.description != resource.description;
        let activities = if changed || !resource.activities.is_empty() {
            serde_json::to_string(&resource.activities)?
        } else {
            previous.activities_json
        };
        diesel::update(analytics_resources::table.find(previous.id))
            .set((
                analytics_resources::description.eq(&resource.description),
                analytics_resources::activities_json.eq(activities),
                analytics_resources::version.eq(&resource.version),
                analytics_resources::origin.eq(resource.origin.as_str()),
            ))
            .execute(&mut self.connection)?;
        Ok(())
    }
}
fn write_exposure(
    connection: &mut crate::persistence::backend::DbConnection,
    row: &AnalyticsExposuresRow,
) -> Result<()> {
    diesel::insert_into(analytics_exposures::table)
        .values(row)
        .on_conflict(analytics_exposures::id)
        .do_update()
        .set((
            analytics_exposures::last_seen.eq(&row.last_seen),
            analytics_exposures::active.eq(true),
        ))
        .execute(connection)?;
    Ok(())
}
