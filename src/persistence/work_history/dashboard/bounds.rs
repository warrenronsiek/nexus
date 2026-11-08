// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// Display and transport bounds; aggregates keep the scanned population.
use super::super::paging::bound_items;
use super::{retention, DashboardProjection, SOURCE_LIMIT};
use anyhow::Result;

impl DashboardProjection {
    pub(super) fn bound_output(&mut self) -> Result<()> {
        let sources = self.bound_sources()?;
        let timeline = self.bound_timeline()?;
        let charts = self.bound_charts()?;
        let transport = self.bound_transport()?;
        if sources || timeline || charts || transport {
            self.view.truncated = true;
            self.view.notices.push("display_limit: chart rows or selectable sessions are partial; duration aggregates retain the scanned population".into());
        }
        Ok(())
    }
    fn bound_sources(&mut self) -> Result<bool> {
        let sessions = bound_items(&mut self.view.sessions, 512, 512 * 1024)?;
        let activities = bound_items(&mut self.view.activities, 512, 128 * 1024)?;
        Ok(sessions || activities)
    }
    fn bound_timeline(&mut self) -> Result<bool> {
        self.view.intervals =
            retention::recent_with_ancestors(std::mem::take(&mut self.view.intervals));
        let intervals = bound_items(&mut self.view.intervals, SOURCE_LIMIT, 512 * 1024)?;
        let mut gaps = false;
        if let Some(accounting) = &mut self.view.accounting {
            gaps = bound_items(&mut accounting.unobserved_intervals, 4097, 64 * 1024)?;
            accounting.coverage.display_partial = intervals || gaps;
        }
        Ok(intervals || gaps)
    }
    fn bound_charts(&mut self) -> Result<bool> {
        let daily = bound_items(&mut self.view.daily, 4096, 64 * 1024)?;
        let kinds = bound_items(&mut self.view.activity_kinds, 4096, 64 * 1024)?;
        let activity_daily = bound_items(&mut self.view.activity_daily, 4096, 96 * 1024)?;
        Ok(daily || kinds || activity_daily)
    }
    fn bound_transport(&mut self) -> Result<bool> {
        let mut trimmed = false;
        while serde_json::to_vec(&self.view)?.len() > 896 * 1024 {
            // Secondary source rows are reconstructible by paginated reads. Preserve
            // the primary waterfall until those rows no longer consume the budget.
            if !self.view.sessions.is_empty() {
                self.view.sessions.truncate(self.view.sessions.len() / 2);
            } else if !self.view.activities.is_empty() {
                self.view
                    .activities
                    .truncate(self.view.activities.len() / 2);
            } else if !self.view.intervals.is_empty() {
                self.view.intervals.truncate(self.view.intervals.len() / 2);
            } else {
                anyhow::bail!("bounded work dashboard metadata exceeds transport budget");
            }
            trimmed = true;
        }
        if trimmed {
            if let Some(accounting) = &mut self.view.accounting {
                accounting.coverage.display_partial = true;
            }
            self.view.notices.push("transport_limit: display rows are partial; exact aggregates retain the selected source population".into());
        }
        Ok(trimmed)
    }
}
