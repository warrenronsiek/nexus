// @feature persistence
// @feature coordination
// @spec docs/features/persistence.md
// @spec docs/features/coordination.md
use super::backend::DbBackend;
use super::schema::{claims, conflicts, sessions};
use chrono::{Duration, Utc};
use diesel::dsl::exists;
use diesel::prelude::*;
use diesel::sql_types::Bool;

type Predicate<Table> = Box<dyn BoxableExpression<Table, DbBackend, SqlType = Bool>>;

/// Activity is observed, not inferred from the lifetime of a multiplexed MCP process.
/// A renewed lease keeps its owner visible until that lease expires.
pub(super) fn recent_session(window_seconds: i64) -> Predicate<sessions::table> {
    let now = Utc::now();
    let cutoff = (now - Duration::seconds(window_seconds)).to_rfc3339();
    Box::new(
        sessions::status.eq("active").and(
            sessions::last_seen_at.gt(cutoff).or(exists(
                claims::table
                    .filter(claims::project_id.eq(sessions::project_id))
                    .filter(claims::session_id.eq(sessions::session_id))
                    .filter(claims::state.eq_any(["claimed", "modified"]))
                    .filter(claims::expires_at.gt(now.to_rfc3339())),
            )),
        ),
    )
}

/// Conflict identity follows the path and session pair, even after leases are replaced.
/// Original claim rows supply identity; current claims supply liveness.
pub(super) fn live_conflict() -> Predicate<conflicts::table> {
    Box::new(
        conflicts::status
            .eq("open")
            .and(live_owner(ConflictOwner::Left))
            .and(live_owner(ConflictOwner::Right)),
    )
}

enum ConflictOwner {
    Left,
    Right,
}

fn live_owner(owner: ConflictOwner) -> Predicate<conflicts::table> {
    let (original, current) = diesel::alias!(claims as original, claims as current);
    let query = original
        .inner_join(
            current.on(current
                .field(claims::project_id)
                .eq(original.field(claims::project_id))
                .and(
                    current
                        .field(claims::session_id)
                        .eq(original.field(claims::session_id)),
                )),
        )
        .filter(original.field(claims::project_id).eq(conflicts::project_id))
        .filter(current.field(claims::path).eq(conflicts::path))
        .filter(current.field(claims::state).eq_any(["claimed", "modified"]))
        .filter(
            current
                .field(claims::expires_at)
                .gt(Utc::now().to_rfc3339()),
        );
    match owner {
        ConflictOwner::Left => Box::new(exists(
            query.filter(original.field(claims::id).eq(conflicts::left_claim_id)),
        )),
        ConflictOwner::Right => Box::new(exists(
            query.filter(original.field(claims::id).eq(conflicts::right_claim_id)),
        )),
    }
}
