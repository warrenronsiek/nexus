// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
use super::*;
use crate::analytics::classification::validate_draft;
use anyhow::{bail, Context};

fn record_id(job: &ClassificationJob) -> String {
    match &job.record {
        ClassificationRecord::Action(id) => id.clone(),
        ClassificationRecord::Resource(id) => format!("resource:{id}"),
    }
}
impl Store {
    pub(crate) fn activity_categories(&mut self) -> Result<Vec<Category>> {
        self.ensure_activity_categories()?;
        let rows = analytics_categories::table
            .filter(analytics_categories::replaced_by.is_null())
            .order(analytics_categories::id.asc())
            .limit(256)
            .load::<AnalyticsCategoriesRow>(&mut self.connection)?;
        Ok(rows
            .into_iter()
            .map(|row| Category {
                id: row.id,
                name: row.name,
                definition: row.definition,
            })
            .collect())
    }
    pub(super) fn ensure_activity_categories(&mut self) -> Result<()> {
        if analytics_categories::table
            .find("ci.inspect")
            .count()
            .get_result::<i64>(&mut self.connection)?
            > 0
        {
            return Ok(());
        }
        let now = Utc::now().to_rfc3339();
        let rows: Vec<_> = crate::analytics::capture::known_categories()
            .into_iter()
            .map(|category| AnalyticsCategoriesRow {
                id: category.id,
                name: category.name,
                definition: category.definition,
                replaced_by: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .collect();
        self.transaction(|connection| {
            for row in rows {
                diesel::insert_into(analytics_categories::table)
                    .values(&row)
                    .on_conflict(analytics_categories::id)
                    .do_nothing()
                    .execute(connection)?;
            }
            Ok(())
        })
    }
    pub(super) fn purpose_for_action(
        &mut self,
        project: &str,
        key: &str,
        action: &Action,
    ) -> Result<Option<ClassifiedPurpose>> {
        if action.category_id != "unclassified" {
            return Ok(None);
        }
        if action.kind == ResourceKind::Tool {
            let resource = analytics_resources::table
                .find(identity(&[project, key]))
                .first::<AnalyticsResourcesRow>(&mut self.connection)
                .optional()?;
            if let Some(resource) = resource {
                let ids: Vec<String> = serde_json::from_str(&resource.activities_json)?;
                if ids.len() == 1 && ids[0] != "unclassified" {
                    return Ok(Some(ClassifiedPurpose {
                        category: ids[0].clone(),
                        description: super::queries::category_name(&ids[0]),
                        evidence: "definition".into(),
                        model: None,
                    }));
                }
                // A changed inventoried definition must not reuse an old name-only label.
                if resource.version.is_some() {
                    return Ok(None);
                }
            }
        }
        self.cached_classification(project, &action.pattern_key)
    }
    fn cached_classification(
        &mut self,
        project: &str,
        pattern: &str,
    ) -> Result<Option<ClassifiedPurpose>> {
        Ok(analytics_actions::table
            .filter(analytics_actions::project_id.eq(project))
            .filter(analytics_actions::pattern_key.eq(pattern))
            .filter(analytics_actions::evidence.eq_any(["model", "manual"]))
            .order(analytics_actions::observed_at.desc())
            .select((
                analytics_actions::category_id,
                analytics_actions::description,
                analytics_actions::evidence,
                analytics_actions::model_id,
            ))
            .first(&mut self.connection)
            .optional()?)
    }
    pub(crate) fn resolve_classification_target(
        &mut self,
        project: &str,
        context: &crate::coordination::domain::HookContext,
        target: &ClassificationTarget,
    ) -> Result<ClassificationJob> {
        let session = identity(&[project, &context.agent, &context.session_id]);
        let job = match target {
            ClassificationTarget::Action {
                invocation_id,
                index,
                ..
            } => self.action_classification(project, &session, invocation_id, *index)?,
            ClassificationTarget::Resource {
                name,
                resource_kind,
                ..
            } => self.resource_classification(project, &session, name, *resource_kind)?,
        };
        if job.pattern_key != target.pattern_key() {
            bail!("classification pattern does not match its subject");
        }
        Ok(job)
    }
    fn action_classification(
        &mut self,
        project: &str,
        session: &str,
        invocation: &str,
        index: u32,
    ) -> Result<ClassificationJob> {
        let call = identity(&[session, invocation]);
        let row = analytics_actions::table
            .find(identity(&[&call, &index.to_string()]))
            .filter(analytics_actions::project_id.eq(project))
            .first::<AnalyticsActionsRow>(&mut self.connection)?;
        Ok(ClassificationJob {
            project_id: project.into(),
            record: ClassificationRecord::Action(row.id),
            pattern_key: row.pattern_key,
        })
    }
    fn resource_classification(
        &mut self,
        project: &str,
        session: &str,
        name: &str,
        kind: ResourceKind,
    ) -> Result<ClassificationJob> {
        // The tool namespace is retained in the exposure key; derive it from the matching session.
        let row = analytics_resources::table
            .inner_join(
                analytics_exposures::table.on(analytics_resources::resource_key
                    .eq(analytics_exposures::resource_key)
                    .and(analytics_resources::project_id.eq(analytics_exposures::project_id))),
            )
            .filter(analytics_resources::project_id.eq(project))
            .filter(analytics_exposures::session_key.eq(session))
            .filter(analytics_resources::name.eq(name))
            .filter(analytics_resources::kind.eq(kind.as_str()))
            .select(analytics_resources::all_columns)
            .first::<AnalyticsResourcesRow>(&mut self.connection)?;
        let expected = resource_pattern(&Resource {
            name: row.name,
            kind,
            origin: Origin::Unknown,
            exposure: Exposure::Declared,
            description: row.description,
            activities: vec![],
            version: row.version,
        });
        Ok(ClassificationJob {
            project_id: project.into(),
            record: ClassificationRecord::Resource(row.id),
            pattern_key: expected,
        })
    }
    pub(crate) fn reserve_activity_classification(
        &mut self,
        job: &ClassificationJob,
        budget: u32,
    ) -> Result<bool> {
        if self.has_classification(job)? {
            return Ok(false);
        }
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let used = analytics_revisions::table
            .filter(analytics_revisions::evidence.eq("attempt"))
            .filter(analytics_revisions::created_at.ge(&today))
            .count()
            .get_result::<i64>(&mut self.connection)?;
        if used >= i64::from(budget) {
            return Ok(false);
        }
        let row = attempt_row(job, &today);
        Ok(diesel::insert_into(analytics_revisions::table)
            .values(&row)
            .on_conflict(analytics_revisions::id)
            .do_nothing()
            .execute(&mut self.connection)?
            == 1)
    }
    fn has_classification(&mut self, job: &ClassificationJob) -> Result<bool> {
        match &job.record {
            ClassificationRecord::Action(id) => Ok(analytics_actions::table
                .find(id)
                .select(analytics_actions::category_id)
                .first::<String>(&mut self.connection)?
                != "unclassified"),
            ClassificationRecord::Resource(id) => {
                let text = analytics_resources::table
                    .find(id)
                    .select(analytics_resources::activities_json)
                    .first::<String>(&mut self.connection)?;
                let categories: Vec<String> = serde_json::from_str(&text)?;
                Ok(categories.iter().any(|id| id != "unclassified"))
            }
        }
    }
    pub(crate) fn apply_activity_classification(
        &mut self,
        job: &ClassificationJob,
        draft: &CategoryDraft,
        model: &str,
    ) -> Result<()> {
        self.validate_reserved_classification(job, draft)?;
        let category = self.canonical_category(&draft.category_id)?;
        let now = Utc::now().to_rfc3339();
        let (definition, revision) = classification_rows(job, draft, model, &category, &now);
        self.connection
            .transaction::<_, anyhow::Error, _>(|connection| {
                diesel::insert_into(analytics_categories::table)
                    .values(&definition)
                    .on_conflict(analytics_categories::id)
                    .do_nothing()
                    .execute(connection)?;
                diesel::insert_into(analytics_revisions::table)
                    .values(&revision)
                    .on_conflict(analytics_revisions::id)
                    .do_nothing()
                    .execute(connection)?;
                project_classification(
                    connection,
                    job,
                    &Purpose {
                        category: &category,
                        description: &draft.description,
                        model,
                    },
                )?;
                Ok(())
            })
    }
    fn validate_reserved_classification(
        &mut self,
        job: &ClassificationJob,
        draft: &CategoryDraft,
    ) -> Result<()> {
        validate_draft(draft)?;
        let today = Utc::now().date_naive();
        let ids: Vec<_> = [today, today - chrono::Duration::days(1)]
            .iter()
            .map(|day| attempt_key(job, &day.to_string()))
            .collect();
        let reserved = analytics_revisions::table
            .filter(analytics_revisions::id.eq_any(ids))
            .filter(analytics_revisions::action_id.eq(record_id(job)))
            .filter(analytics_revisions::evidence.eq("attempt"))
            .count()
            .get_result::<i64>(&mut self.connection)?;
        if reserved == 0 {
            bail!("classification has no matching reserved attempt");
        }
        Ok(())
    }
    pub(super) fn canonical_category(&mut self, id: &str) -> Result<String> {
        let mut current = id.to_owned();
        for _ in 0..16 {
            let replacement = analytics_categories::table
                .find(&current)
                .select(analytics_categories::replaced_by)
                .first::<Option<String>>(&mut self.connection)
                .optional()?
                .flatten();
            match replacement {
                Some(next) => current = next,
                None => return Ok(current),
            }
        }
        bail!("category redirect chain exceeds limit")
    }
    pub(crate) fn rename_activity_category(
        &mut self,
        id: &str,
        name: &str,
        definition: &str,
    ) -> Result<()> {
        validate_draft(&CategoryDraft {
            category_id: id.into(),
            name: name.into(),
            definition: definition.into(),
            description: name.into(),
        })?;
        self.connection
            .transaction::<_, anyhow::Error, _>(|connection| {
                let changed = diesel::update(
                    analytics_categories::table
                        .find(id)
                        .filter(analytics_categories::replaced_by.is_null()),
                )
                .set((
                    analytics_categories::name.eq(name),
                    analytics_categories::definition.eq(definition),
                    analytics_categories::updated_at.eq(Utc::now().to_rfc3339()),
                ))
                .execute(connection)?;
                if changed != 1 {
                    bail!("category does not exist or was merged");
                }
                record_taxonomy_revision(
                    connection,
                    id,
                    &format!("Renamed to {name}: {definition}"),
                )?;
                Ok(())
            })
    }
    pub(crate) fn merge_activity_categories(&mut self, source: &str, target: &str) -> Result<()> {
        let target = self.canonical_category(target)?;
        if source == target {
            bail!("cannot merge a category into itself");
        }
        analytics_categories::table
            .find(&target)
            .first::<AnalyticsCategoriesRow>(&mut self.connection)
            .context("target category must exist")?;
        self.connection
            .transaction::<_, anyhow::Error, _>(|connection| {
                let changed = diesel::update(
                    analytics_categories::table
                        .find(source)
                        .filter(analytics_categories::replaced_by.is_null()),
                )
                .set((
                    analytics_categories::replaced_by.eq(Some(&target)),
                    analytics_categories::updated_at.eq(Utc::now().to_rfc3339()),
                ))
                .execute(connection)?;
                if changed != 1 {
                    bail!("source category does not exist or was merged");
                }
                diesel::update(
                    analytics_actions::table.filter(analytics_actions::category_id.eq(source)),
                )
                .set(analytics_actions::category_id.eq(&target))
                .execute(connection)?;
                record_taxonomy_revision(connection, source, &format!("Merged into {target}"))?;
                Ok(())
            })
    }
    pub(crate) fn mark_activity_classification_failed(
        &mut self,
        job: &ClassificationJob,
    ) -> Result<()> {
        if let ClassificationRecord::Action(id) = &job.record {
            diesel::update(
                analytics_actions::table
                    .find(id)
                    .filter(analytics_actions::project_id.eq(&job.project_id))
                    .filter(analytics_actions::evidence.eq("pending")),
            )
            .set(analytics_actions::evidence.eq("failed"))
            .execute(&mut self.connection)?;
        }
        Ok(())
    }
}

fn attempt_row(job: &ClassificationJob, today: &str) -> AnalyticsRevisionsRow {
    AnalyticsRevisionsRow {
        id: attempt_key(job, today),
        action_id: record_id(job),
        category_id: "unclassified".into(),
        description: "Classification requested".into(),
        evidence: "attempt".into(),
        model_id: None,
        policy_version: POLICY_VERSION.into(),
        created_at: Utc::now().to_rfc3339(),
    }
}

fn classification_rows(
    job: &ClassificationJob,
    draft: &CategoryDraft,
    model: &str,
    category: &str,
    now: &str,
) -> (AnalyticsCategoriesRow, AnalyticsRevisionsRow) {
    let definition = AnalyticsCategoriesRow {
        id: category.into(),
        name: draft.name.clone(),
        definition: draft.definition.clone(),
        replaced_by: None,
        created_at: now.into(),
        updated_at: now.into(),
    };
    let revision = AnalyticsRevisionsRow {
        id: identity(&[
            &record_id(job),
            &job.pattern_key,
            category,
            &draft.description,
            model,
            POLICY_VERSION,
        ]),
        action_id: record_id(job),
        category_id: category.into(),
        description: draft.description.clone(),
        evidence: "model".into(),
        model_id: Some(model.into()),
        policy_version: POLICY_VERSION.into(),
        created_at: now.into(),
    };
    (definition, revision)
}

struct Purpose<'a> {
    category: &'a str,
    description: &'a str,
    model: &'a str,
}
fn project_classification(
    connection: &mut crate::persistence::backend::DbConnection,
    job: &ClassificationJob,
    purpose: &Purpose<'_>,
) -> Result<()> {
    match &job.record {
        ClassificationRecord::Action(id) => {
            let version = analytics_actions::table
                .find(id)
                .select(analytics_actions::resource_version)
                .first::<Option<String>>(connection)?;
            apply_purpose(
                connection,
                &job.project_id,
                Subject::Pattern(&job.pattern_key),
                purpose,
                version,
            )?;
        }
        ClassificationRecord::Resource(id) => project_definition(connection, job, id, purpose)?,
    }
    Ok(())
}
fn project_definition(
    connection: &mut crate::persistence::backend::DbConnection,
    job: &ClassificationJob,
    id: &str,
    purpose: &Purpose<'_>,
) -> Result<()> {
    diesel::update(analytics_resources::table.find(id))
        .set(analytics_resources::activities_json.eq(serde_json::to_string(&[purpose.category])?))
        .execute(connection)?;
    let (key, version) = analytics_resources::table
        .find(id)
        .select((
            analytics_resources::resource_key,
            analytics_resources::version,
        ))
        .first::<(String, Option<String>)>(connection)?;
    apply_purpose(
        connection,
        &job.project_id,
        Subject::Definition(&key),
        purpose,
        version,
    )
}
enum Subject<'a> {
    Pattern(&'a str),
    Definition(&'a str),
}
fn apply_purpose(
    connection: &mut crate::persistence::backend::DbConnection,
    project: &str,
    subject: Subject<'_>,
    purpose: &Purpose<'_>,
    version: Option<String>,
) -> Result<()> {
    let target = diesel::update(analytics_actions::table)
        .filter(analytics_actions::project_id.eq(project))
        .filter(analytics_actions::category_id.eq("unclassified"))
        .into_boxed::<DbBackend>();
    let (evidence, mut target) = match subject {
        Subject::Pattern(key) => (
            "model",
            target.filter(analytics_actions::pattern_key.eq(key)),
        ),
        Subject::Definition(key) => (
            "definition",
            target
                .filter(analytics_actions::resource_key.eq(key))
                .filter(analytics_actions::kind.eq("tool")),
        ),
    };
    target = match version {
        Some(version) => target.filter(analytics_actions::resource_version.eq(version)),
        None => target.filter(analytics_actions::resource_version.is_null()),
    };
    target
        .set((
            analytics_actions::category_id.eq(purpose.category),
            analytics_actions::description.eq(purpose.description),
            analytics_actions::evidence.eq(evidence),
            analytics_actions::model_id.eq(Some(purpose.model)),
        ))
        .execute(connection)?;
    Ok(())
}

fn attempt_key(job: &ClassificationJob, day: &str) -> String {
    identity(&[&job.project_id, &job.pattern_key, POLICY_VERSION, day])
}

fn record_taxonomy_revision(
    connection: &mut crate::persistence::backend::DbConnection,
    category: &str,
    description: &str,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let row = AnalyticsRevisionsRow {
        id: identity(&[category, description, &now]),
        action_id: format!("category:{category}"),
        category_id: category.into(),
        description: description.into(),
        evidence: "manual".into(),
        model_id: None,
        policy_version: POLICY_VERSION.into(),
        created_at: now,
    };
    diesel::insert_into(analytics_revisions::table)
        .values(&row)
        .execute(connection)?;
    Ok(())
}
