// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use crate::persistence::window::WindowBudget;
use anyhow::{bail, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cursor {
    pub after: String,
    pub cutoff: DateTime<Utc>,
    scope: String,
}
impl Cursor {
    fn open(after: Option<&str>, scope: String) -> Result<Self> {
        if let Some(value) = after {
            if value.len() > 4096 {
                bail!("work cursor exceeds limit");
            }
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value)?;
            let cursor: Self = serde_json::from_slice(&bytes)?;
            if cursor.scope != scope {
                bail!("work cursor belongs to another query");
            }
            Ok(cursor)
        } else {
            Ok(Self {
                after: String::new(),
                cutoff: Utc::now(),
                scope,
            })
        }
    }
    fn next(&self, after: String) -> Result<String> {
        Ok(
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Self {
                after,
                cutoff: self.cutoff,
                scope: self.scope.clone(),
            })?),
        )
    }
}
pub(super) struct Page<T> {
    pub cursor: Cursor,
    pub limit: i64,
    items: Vec<T>,
    last: Option<String>,
    budget: WindowBudget,
    stopped: bool,
}
pub(super) struct PageResult<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}
impl<T: Serialize> Page<T> {
    pub fn open(after: Option<&str>, scope: String, limit: Option<u32>) -> Result<Self> {
        Ok(Self {
            cursor: Cursor::open(after, scope)?,
            limit: limit.unwrap_or(24).clamp(1, 64) as i64,
            items: vec![],
            last: None,
            budget: WindowBudget::new(limit.unwrap_or(24).clamp(1, 64) as usize, 512 * 1024),
            stopped: false,
        })
    }
    pub fn include(&mut self, id: String, item: Option<T>) -> Result<bool> {
        if let Some(item) = item {
            if !self.budget.include(&item)? {
                self.stopped = true;
                return Ok(false);
            }
            self.items.push(item);
        }
        self.last = Some(id);
        Ok(true)
    }
    pub fn finish(self, more: bool) -> Result<PageResult<T>> {
        let next_cursor = if more || self.stopped {
            self.last.map(|id| self.cursor.next(id)).transpose()?
        } else {
            None
        };
        Ok(PageResult {
            items: self.items,
            next_cursor,
            truncated: self.stopped,
        })
    }
}
