// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
// @entrypoint Store::skill_detail
//! Latest observed skill manifest text, keyed by project and skill resource.
use super::*;
use crate::analytics::skills::SkillDocument;

impl Store {
    pub(crate) fn record_skill_document(
        &mut self,
        project: &str,
        name: &str,
        document: &SkillDocument,
    ) -> Result<()> {
        document.validate()?;
        let resource_key = resource_key(ResourceKind::Skill, name, "");
        let id = identity(&[project, &resource_key]);
        let content_hash = document.content_hash();
        let current = analytics_skill_documents::table
            .find(&id)
            .select(analytics_skill_documents::content_hash)
            .first::<String>(&mut self.connection)
            .optional()?;
        // Agents reread the same manifest constantly; only a changed version is written.
        if current.as_deref() == Some(content_hash.as_str()) {
            return Ok(());
        }
        let row = AnalyticsSkillDocumentsRow {
            id,
            project_id: project.into(),
            resource_key,
            name: name.into(),
            description: document.description(),
            content: document.content.clone(),
            content_hash,
            source_path: document.source_path.clone(),
            observed_at: Utc::now().to_rfc3339(),
        };
        diesel::insert_into(analytics_skill_documents::table)
            .values(&row)
            .on_conflict(analytics_skill_documents::id)
            .do_update()
            .set((
                analytics_skill_documents::name.eq(&row.name),
                analytics_skill_documents::description.eq(&row.description),
                analytics_skill_documents::content.eq(&row.content),
                analytics_skill_documents::content_hash.eq(&row.content_hash),
                analytics_skill_documents::source_path.eq(&row.source_path),
                analytics_skill_documents::observed_at.eq(&row.observed_at),
            ))
            .execute(&mut self.connection)?;
        Ok(())
    }

    /// The newest captured text for a skill; a global read selects across projects.
    pub(crate) fn skill_detail(
        &mut self,
        project: Option<&str>,
        name: &str,
    ) -> Result<SkillDetail> {
        let key = resource_key(ResourceKind::Skill, name, "");
        let mut documents = analytics_skill_documents::table
            .filter(analytics_skill_documents::resource_key.eq(&key))
            .select((
                analytics_skill_documents::description,
                analytics_skill_documents::content,
                analytics_skill_documents::content_hash,
                analytics_skill_documents::source_path,
                analytics_skill_documents::observed_at,
            ))
            .order((
                analytics_skill_documents::observed_at.desc(),
                analytics_skill_documents::project_id.asc(),
            ))
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            documents = documents.filter(analytics_skill_documents::project_id.eq(project));
        }
        let document = documents
            .first::<(Option<String>, String, String, String, String)>(&mut self.connection)
            .optional()?;
        let Some((description, content, content_hash, source_path, observed_at)) = document else {
            return Ok(SkillDetail {
                ok: true,
                name: name.into(),
                description: self.inventoried_skill_description(project, &key)?,
                content: None,
                content_hash: None,
                source_path: None,
                observed_at: None,
            });
        };
        Ok(SkillDetail {
            ok: true,
            name: name.into(),
            description,
            content: Some(content),
            content_hash: Some(content_hash),
            source_path: Some(source_path),
            observed_at: Some(observed_at),
        })
    }

    fn inventoried_skill_description(
        &mut self,
        project: Option<&str>,
        key: &str,
    ) -> Result<Option<String>> {
        let mut query = analytics_resources::table
            .filter(analytics_resources::resource_key.eq(key))
            .filter(analytics_resources::description.ne(""))
            .select(analytics_resources::description)
            .order(analytics_resources::last_seen.desc())
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            query = query.filter(analytics_resources::project_id.eq(project));
        }
        Ok(query.first::<String>(&mut self.connection).optional()?)
    }
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
