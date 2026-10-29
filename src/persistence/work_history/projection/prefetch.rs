// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::*;

impl Projection<'_> {
    /// Cache only complete subject groups; a cut or oversized group uses the ordinary read.
    pub fn prefetch(&mut self, subjects: &[WorkSubject]) -> Result<()> {
        self.observations.clear();
        let sessions: Vec<_> = subjects
            .iter()
            .filter(|s| s.scope() == "session")
            .map(|s| s.id())
            .collect();
        let activities: Vec<_> = subjects
            .iter()
            .filter(|s| s.scope() == "activity")
            .map(|s| s.id())
            .collect();
        let rows = work_observations::table
            .filter(work_observations::project_id.eq(self.project))
            .filter(
                work_observations::subject_scope
                    .eq("session")
                    .and(work_observations::subject_id.eq_any(sessions))
                    .or(work_observations::subject_scope
                        .eq("activity")
                        .and(work_observations::subject_id.eq_any(activities))),
            )
            .order((
                work_observations::subject_scope.asc(),
                work_observations::subject_id.asc(),
                work_observations::id.asc(),
            ))
            .limit(PROJECTION_LIMIT + 1)
            .select(ObservationRow::as_select())
            .load::<ObservationRow>(self.connection)?;
        self.cache_observation_page(subjects, rows)?;
        self.prefetch_children(subjects)
    }
    fn prefetch_children(&mut self, subjects: &[WorkSubject]) -> Result<()> {
        self.children = subjects
            .iter()
            .cloned()
            .map(|subject| (subject, false))
            .collect();
        let targets = work_links::table
            .filter(work_links::project_id.eq(self.project))
            .filter(work_links::target_id.eq_any(subjects.iter().map(|subject| subject.id())))
            .filter(
                work_links::target_scope
                    .eq("session")
                    .and(work_links::relation.eq("member_of"))
                    .or(work_links::target_scope
                        .eq("activity")
                        .and(work_links::relation.eq("parent"))),
            )
            .select((work_links::target_scope, work_links::target_id))
            .distinct()
            .limit(129)
            .load::<(String, String)>(self.connection)?;
        for (scope, id) in targets {
            let subject = if scope == "session" {
                WorkSubject::Session { id }
            } else {
                WorkSubject::Activity { id }
            };
            if let Some(value) = self.children.get_mut(&subject) {
                *value = true;
            }
        }
        Ok(())
    }
    fn cache_observation_page(
        &mut self,
        subjects: &[WorkSubject],
        rows: Vec<ObservationRow>,
    ) -> Result<()> {
        let incomplete = if rows.len() == (PROJECTION_LIMIT + 1) as usize {
            Some(rows.last().unwrap().observation()?.input.subject)
        } else {
            None
        };
        let mut groups: HashMap<WorkSubject, Vec<WorkObservation>> = HashMap::new();
        for row in rows {
            let observation = row.observation()?;
            groups
                .entry(observation.input.subject.clone())
                .or_default()
                .push(observation);
        }
        if incomplete.is_none() {
            for subject in subjects {
                groups.entry(subject.clone()).or_default();
            }
        }
        let mut bytes = 0;
        for (subject, observations) in groups {
            if incomplete.as_ref() == Some(&subject) {
                continue;
            }
            bytes += serde_json::to_vec(&(&subject, &observations))?.len();
            if bytes <= 8 * 1024 * 1024 {
                self.observations.insert(subject, observations);
            }
        }
        Ok(())
    }
}
