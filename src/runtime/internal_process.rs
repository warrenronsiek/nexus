// @feature runtime
// @feature work-history
// @feature usage-analytics
// @spec docs/features/runtime.md
// @spec docs/features/work-history.md
// @spec docs/features/usage-analytics.md
// Marks host processes that Nexus itself spawns for its own model work.
//
// Reviewers, memory summarizers, analysts, classifiers, and title readers are
// Nexus's own provider calls rather than user sessions. They inherit this marker
// so every lifecycle entry point in their process tree can decline to record
// coordination, work capture, or analytics while still answering permissively.
use crate::coordination::domain::HookResponse;
use std::ffi::OsStr;

/// Environment variable carried by every provider process Nexus spawns.
pub const INTERNAL_PROCESS_ENV: &str = "NEXUS_INTERNAL_PROCESS";
/// Value Nexus assigns; any nonempty value marks the process as internal.
pub const INTERNAL_PROCESS_VALUE: &str = "1";
/// Diagnostic returned by lifecycle entry points that intentionally record nothing.
pub const INTERNAL_PROCESS_DIAGNOSTIC: &str =
    "nexus-internal provider process; lifecycle evidence is not recorded";

/// True when this process descends from a Nexus-spawned provider.
pub fn active() -> bool {
    marked(std::env::var_os(INTERNAL_PROCESS_ENV).as_deref())
}

fn marked(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

/// Permissive lifecycle response that makes no coordination or capture claim.
pub fn unrecorded_hook_response() -> HookResponse {
    HookResponse {
        permitted: true,
        recorded: false,
        retryable: false,
        advisories: Vec::new(),
        diagnostic: Some(INTERNAL_PROCESS_DIAGNOSTIC.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_nonempty_marker_identifies_an_internal_process() {
        assert!(!marked(None));
        assert!(!marked(Some(OsStr::new(""))));
        assert!(marked(Some(OsStr::new(INTERNAL_PROCESS_VALUE))));
    }

    #[test]
    fn internal_lifecycle_responses_permit_without_claiming_a_record() {
        let response = unrecorded_hook_response();
        assert!(response.permitted);
        assert!(!response.recorded);
        assert!(!response.retryable);
        assert!(response.advisories.is_empty());
    }
}
