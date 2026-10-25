// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::timeline::{self, TimelineRequest};
use crate::persistence::backend::DbConnection;
use crate::work_history::{WorkResponse, WorkSummary};
use anyhow::Result;
use std::collections::BTreeSet;
pub(super) struct SessionActivities {
    pub items: Vec<WorkSummary>,
    pub truncated: bool,
}
struct Selection {
    result: SessionActivities,
    parents: Vec<Option<String>>,
    seen: BTreeSet<String>,
    bytes: usize,
    pages: usize,
}
pub(super) fn activities(
    connection: &mut DbConnection,
    project: &str,
    session: &str,
) -> Result<SessionActivities> {
    let mut selection = Selection {
        result: SessionActivities {
            items: vec![],
            truncated: false,
        },
        parents: vec![None],
        seen: BTreeSet::new(),
        bytes: 0,
        pages: 0,
    };
    while let Some(parent) = selection.parents.pop() {
        if !selection.children(connection, project, session, parent.as_deref())? {
            break;
        }
    }
    Ok(selection.result)
}
impl Selection {
    fn children(
        &mut self,
        connection: &mut DbConnection,
        project: &str,
        session: &str,
        parent: Option<&str>,
    ) -> Result<bool> {
        let mut after = None;
        loop {
            if self.pages == 64 {
                self.result.truncated = true;
                return Ok(false);
            }
            self.pages += 1;
            let response = timeline::timeline(
                connection,
                project,
                TimelineRequest {
                    session,
                    parent,
                    after: after.as_deref(),
                    limit: Some(64),
                },
            )?;
            let WorkResponse::Timeline {
                items,
                next_cursor,
                truncated,
                ..
            } = response
            else {
                unreachable!("typed timeline result");
            };
            self.result.truncated |= truncated;
            for item in items {
                if !self.include(item)? {
                    return Ok(false);
                }
            }
            after = next_cursor;
            if after.is_none() {
                return Ok(true);
            }
        }
    }
    fn include(&mut self, item: WorkSummary) -> Result<bool> {
        if !self.seen.insert(item.subject.id().to_owned()) {
            return Ok(true);
        }
        self.bytes += serde_json::to_vec(&item)?.len();
        if self.result.items.len() == super::dashboard::SOURCE_LIMIT
            || self.bytes > super::dashboard::SOURCE_BYTES
        {
            self.result.truncated = true;
            return Ok(false);
        }
        if item.has_children {
            self.parents.push(Some(item.subject.id().into()));
        }
        self.result.items.push(item);
        Ok(true)
    }
}
