// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::*;
use crate::persistence::batch::{complete_groups, GroupBounds};

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
        let has_children = diesel::dsl::exists(
            work_links::table
                .filter(work_links::project_id.eq(work_subjects::project_id))
                .filter(work_links::target_scope.eq(work_subjects::scope))
                .filter(work_links::target_id.eq(work_subjects::id))
                .filter(
                    work_links::target_scope
                        .eq("session")
                        .and(work_links::relation.eq("member_of"))
                        .or(work_links::target_scope
                            .eq("activity")
                            .and(work_links::relation.eq("parent"))),
                ),
        );
        let targets = work_subjects::table
            .filter(work_subjects::project_id.eq(self.project))
            .filter(work_subjects::scope.eq_any(["session", "activity"]))
            .filter(work_subjects::id.eq_any(subjects.iter().map(|subject| subject.id())))
            .select((work_subjects::scope, work_subjects::id, has_children))
            .limit(129)
            .load::<(String, String, bool)>(self.connection)?;
        self.cache_child_flags(targets);
        Ok(())
    }
    fn cache_child_flags(&mut self, targets: Vec<(String, String, bool)>) {
        for (scope, id, exists) in targets {
            let subject = if scope == "session" {
                WorkSubject::Session { id }
            } else {
                WorkSubject::Activity { id }
            };
            if let Some(value) = self.children.get_mut(&subject) {
                *value = exists;
            }
        }
    }
    fn cache_observation_page(
        &mut self,
        subjects: &[WorkSubject],
        rows: Vec<ObservationRow>,
    ) -> Result<()> {
        let observations = rows
            .into_iter()
            .map(|row| {
                let observation = row.observation()?;
                Ok((observation.input.subject.clone(), observation))
            })
            .collect::<Result<Vec<_>>>()?;
        self.observations = complete_groups(
            observations,
            subjects.iter().cloned(),
            GroupBounds {
                rows: (PROJECTION_LIMIT + 1) as usize,
                bytes: 8 * 1024 * 1024,
                items: PROJECTION_LIMIT as usize,
            },
        )?
        .into_iter()
        .collect();
        Ok(())
    }
}
