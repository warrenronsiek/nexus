// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use crate::persistence::backend::DbConnection;
use crate::work_history::*;
use anyhow::Result;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Default)]
pub(super) struct Diagnostics {
    pub conflicts: Vec<String>,
    pub gaps: Vec<String>,
}
impl Diagnostics {
    pub fn merge(&mut self, other: Self) {
        self.conflicts.extend(other.conflicts);
        self.gaps.extend(other.gaps);
    }
}
struct Corrections<'a, 'b> {
    observations: &'a [WorkObservation],
    by_id: HashMap<&'a str, &'a WorkObservation>,
    connection: &'b mut DbConnection,
    project: &'b str,
    diagnostics: &'b mut Diagnostics,
}
pub(super) fn effective<'a>(
    connection: &mut DbConnection,
    project: &str,
    observations: &'a [WorkObservation],
    diagnostics: &mut Diagnostics,
) -> Result<Vec<&'a WorkObservation>> {
    Corrections {
        observations,
        by_id: observations
            .iter()
            .map(|item| (item.input.id.as_str(), item))
            .collect(),
        connection,
        project,
        diagnostics,
    }
    .select()
}
impl<'a> Corrections<'a, '_> {
    fn select(mut self) -> Result<Vec<&'a WorkObservation>> {
        let mut superseded = HashSet::new();
        let mut invalid = HashSet::new();
        let mut children: HashMap<&str, usize> = HashMap::new();
        for item in self.observations {
            if let Some(correction) = &item.input.correction {
                if self.valid(item)? {
                    superseded.insert(correction.replaces.as_str());
                    *children.entry(correction.replaces.as_str()).or_default() += 1;
                } else {
                    invalid.insert(item.input.id.as_str());
                }
            }
        }
        if children.values().any(|count| *count > 1) {
            self.diagnostics
                .conflicts
                .push("competing_corrections".into());
        }
        Ok(self
            .observations
            .iter()
            .filter(|item| {
                !superseded.contains(item.input.id.as_str())
                    && !invalid.contains(item.input.id.as_str())
            })
            .collect())
    }
    fn valid(&mut self, item: &WorkObservation) -> Result<bool> {
        let mut current = item;
        let mut seen = HashSet::new();
        while let Some(correction) = &current.input.correction {
            if !seen.insert(current.input.id.as_str()) {
                self.diagnostics.conflicts.push("correction_cycle".into());
                return Ok(false);
            }
            let Some(previous) = self.by_id.get(correction.replaces.as_str()) else {
                return self.missing(item, &correction.replaces);
            };
            if !compatible(previous, item) {
                self.diagnostics
                    .conflicts
                    .push("invalid_correction_scope".into());
                return Ok(false);
            }
            current = previous;
        }
        Ok(true)
    }
    fn missing(&mut self, item: &WorkObservation, id: &str) -> Result<bool> {
        let external = crate::persistence::lookup::optional::<super::rows::ObservationRow, _, _>(
            self.connection,
            crate::persistence::schema::work_observations::table,
            (self.project, id),
        )?
        .map(|row| row.observation())
        .transpose()?;
        if external.is_some_and(|previous| !compatible(&previous, item)) {
            self.diagnostics
                .conflicts
                .push("invalid_correction_scope".into());
        } else {
            self.diagnostics.gaps.push("unresolved_correction".into());
        }
        Ok(false)
    }
}
fn compatible(left: &WorkObservation, right: &WorkObservation) -> bool {
    left.input.subject == right.input.subject
        && left.input.body.family() == right.input.body.family()
}
