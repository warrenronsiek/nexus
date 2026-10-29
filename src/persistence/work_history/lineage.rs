// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::corrections::Diagnostics;
use super::projection::Projection;
use crate::work_history::*;
use anyhow::Result;
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct Lineage {
    pub sessions: Vec<String>,
    pub parent: Option<String>,
    pub diagnostics: Diagnostics,
}
impl Lineage {
    pub fn resolve(projection: &mut Projection<'_>, subject: &WorkSubject) -> Result<Self> {
        let mut lineage = Self::default();
        let mut seen = HashSet::new();
        let mut current = subject.clone();
        for depth in 0..64 {
            if !seen.insert(current.clone()) {
                lineage.diagnostics.conflicts.push("parent_cycle".into());
                lineage.parent = None;
                break;
            }
            let (links, diagnostics) = projection.active_links(&current)?;
            lineage.diagnostics.merge(diagnostics);
            let Some(parent) = lineage.include(links) else {
                break;
            };
            if depth == 0 {
                lineage.parent = Some(parent.clone());
            }
            current = WorkSubject::Activity { id: parent };
            if depth == 63 {
                lineage.diagnostics.gaps.push("lineage_limit".into());
                lineage.parent = None;
            }
        }
        lineage.sessions.sort();
        lineage.sessions.dedup();
        if lineage.sessions.len() > 64 {
            lineage.sessions.truncate(64);
            lineage.diagnostics.gaps.push("membership_limit".into());
        }
        Ok(lineage)
    }
    fn include(&mut self, links: Vec<WorkLink>) -> Option<String> {
        let mut parents = HashSet::new();
        for link in links {
            match (&*link.relation, link.target) {
                ("member_of", WorkReference::Session { id }) => self.sessions.push(id),
                ("parent", WorkReference::Activity { id }) => {
                    parents.insert(id);
                }
                _ => {}
            }
        }
        if parents.len() > 1 {
            self.diagnostics.conflicts.push("competing_parents".into());
            self.parent = None;
            None
        } else {
            parents.into_iter().next()
        }
    }
}
