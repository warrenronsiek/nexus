// @feature coordination
// @spec docs/features/coordination.md
use super::{SessionRecord, Severity};
use chrono::{DateTime, Utc};
use serde::Serialize;

/// The other party of an overlap, named for people reading coordination history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionCounterpart {
    pub session_id: String,
    pub agent: String,
    pub display_name: Option<String>,
}

impl SessionCounterpart {
    const SHORT_ID_CHARS: usize = 8;

    /// A counterpart whose session record is unavailable.
    pub fn unnamed(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_owned(),
            agent: "unknown".to_owned(),
            display_name: None,
        }
    }

    /// The human-readable subject used in advisory messages.
    pub fn label(&self) -> String {
        let name = self
            .display_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&self.agent);
        let short_id: String = self.session_id.chars().take(Self::SHORT_ID_CHARS).collect();
        format!("{name} (session {short_id})")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineEntryKind {
    ConflictDetected,
    Advisory,
    ConflictResolved,
}

/// `Received` means Nexus told the timeline's session; `Sent` means Nexus told the
/// counterpart about this session's claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineDirection {
    Received,
    Sent,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineEntry {
    pub id: String,
    pub at: DateTime<Utc>,
    #[serde(rename = "type")]
    pub kind: TimelineEntryKind,
    pub path: String,
    pub severity: Severity,
    pub message: String,
    pub direction: TimelineDirection,
    pub counterpart: Option<SessionCounterpart>,
    pub conflict_id: Option<String>,
    pub status: Option<super::ConflictStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionTimeline {
    /// Absent when the project has no session with the requested identity.
    pub session: Option<SessionRecord>,
    /// Oldest first.
    pub entries: Vec<TimelineEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionActivity {
    #[serde(flatten)]
    pub session: SessionRecord,
    pub last_activity_at: DateTime<Utc>,
    /// Conflicts in which this session owns either claim.
    pub conflicts: i64,
    /// Advisories Nexus delivered to this session.
    pub advisories: i64,
}

#[cfg(test)]
mod tests {
    use super::SessionCounterpart;

    #[test]
    fn counterpart_labels_prefer_display_names_and_shorten_session_ids() {
        let mut counterpart = SessionCounterpart {
            session_id: "3f2a9c1b-7d1e-4b6a".into(),
            agent: "codex".into(),
            display_name: None,
        };
        assert_eq!(counterpart.label(), "codex (session 3f2a9c1b)");
        counterpart.display_name = Some("Implement billing".into());
        assert_eq!(counterpart.label(), "Implement billing (session 3f2a9c1b)");
        counterpart.session_id = "é1".into();
        assert_eq!(counterpart.label(), "Implement billing (session é1)");
    }
}
