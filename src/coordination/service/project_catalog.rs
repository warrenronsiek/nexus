// @feature coordination
// @feature observability-ui
// @spec docs/features/coordination.md
// @spec docs/features/observability-ui.md
// Project discovery reads one small catalog instead of aggregating every history table.
// Recording is advisory: it never waits for a busy store and never fails the request.
use super::NexusService;
use crate::coordination::workspace::{self, ProjectBinding, ProjectIdentity};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Recency only orders the project picker, so per-minute precision avoids a write per hook.
const ACTIVITY_REFRESH: Duration = Duration::from_secs(60);
const MAX_REMEMBERED_PROJECTS: usize = 4096;

#[derive(Default)]
pub(super) struct ProjectCatalog {
    recorded: Mutex<HashMap<String, Recorded>>,
}

struct Recorded {
    at: Instant,
    display_name: Option<String>,
}

impl ProjectCatalog {
    fn due(&self, project: &ProjectIdentity, now: Instant) -> bool {
        let Ok(recorded) = self.recorded.lock() else {
            return false;
        };
        recorded.get(&project.id).is_none_or(|previous| {
            now.duration_since(previous.at) >= ACTIVITY_REFRESH
                || (project.display_name.is_some() && project.display_name != previous.display_name)
        })
    }

    fn remember(&self, project: &ProjectIdentity, now: Instant) {
        let Ok(mut recorded) = self.recorded.lock() else {
            return;
        };
        if recorded.len() >= MAX_REMEMBERED_PROJECTS && !recorded.contains_key(&project.id) {
            recorded.clear();
        }
        let display_name = project.display_name.clone().or_else(|| {
            recorded
                .get(&project.id)
                .and_then(|previous| previous.display_name.clone())
        });
        recorded.insert(
            project.id.clone(),
            Recorded {
                at: now,
                display_name,
            },
        );
    }
}

impl NexusService {
    /// Resolves the project a request acts in and records agent activity there.
    pub(in crate::coordination) fn resolve_project(
        &self,
        binding: &ProjectBinding,
        project_root: Option<&str>,
    ) -> Result<ProjectIdentity> {
        let project = workspace::identify(binding, project_root)?;
        if binding.records_activity() {
            self.record_project_activity(&project);
        }
        Ok(project)
    }

    fn record_project_activity(&self, project: &ProjectIdentity) {
        let now = Instant::now();
        if !self.project_catalog.due(project, now) {
            return;
        }
        // Fails open like a lifecycle hook; the next due request retries.
        let Ok(mut store) = self.lifecycle_store() else {
            return;
        };
        let recorded = store.record_project_activity(
            &project.id,
            project.display_name.as_deref(),
            chrono::Utc::now(),
        );
        drop(store);
        if recorded.is_ok() {
            self.project_catalog.remember(project, now);
        }
    }
}
