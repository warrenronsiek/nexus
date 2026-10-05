// @feature agent-memory
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/persistence.md
use super::compaction::validate_compaction_job;
use super::*;

impl Store {
    pub fn record_memory_compaction_attempt(
        &mut self,
        job: &MemoryCompactionJob,
        attempt: &MemoryCompactionAttempt,
    ) -> Result<()> {
        validate_compaction_job(job)?;
        if attempt.provider.trim().is_empty() {
            bail!("memory compaction attempt requires a provider");
        }
        self.connection
            .immediate_transaction::<_, anyhow::Error, _>(|connection| {
                let prior = latest_attempt(connection, job, &attempt.provider)?;
                let prepared = prepare_attempt(attempt, prior.as_ref());
                persist_attempt(connection, job, attempt, &prepared)?;
                queue::update_provider_retry(
                    connection,
                    job,
                    &attempt.provider,
                    prepared.next_retry_at.as_deref(),
                )
            })
    }

    pub fn memory_health(
        &mut self,
        project_id: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<MemoryHealth> {
        let spaces = selected_spaces(&mut self.connection, project_id)?;
        let snapshot = load_health_snapshot(&mut self.connection, &spaces, project_id, now)?;
        finalize_health(snapshot)
    }
}

struct PreparedAttempt {
    id: String,
    outcome: String,
    diagnostic: Option<String>,
    consecutive_failures: i32,
    attempted_at: String,
    next_retry_at: Option<String>,
}

struct HealthSnapshot {
    scopes: Vec<MemoryScopeHealth>,
    latest_attempts: Vec<MemoryCompactionAttemptRow>,
    active_retries: BTreeMap<String, DateTime<Utc>>,
    pending_summaries: usize,
    failed_attempts: usize,
    cooling_down_attempts: usize,
    fallback_uses: usize,
    degraded: bool,
    last_activation_at: Option<DateTime<Utc>>,
}

struct QueueHealthSnapshot {
    active_retries: BTreeMap<String, DateTime<Utc>>,
    pending_summaries: usize,
    cooling_down_attempts: usize,
    degraded: bool,
}

struct AttemptHealthSnapshot {
    latest_attempts: Vec<MemoryCompactionAttemptRow>,
    failed_attempts: usize,
    fallback_uses: usize,
}

fn prepare_attempt(
    attempt: &MemoryCompactionAttempt,
    prior: Option<&MemoryCompactionAttemptRow>,
) -> PreparedAttempt {
    let consecutive_failures = consecutive_failure_count(attempt.outcome, prior);
    PreparedAttempt {
        id: Uuid::new_v4().to_string(),
        outcome: attempt.outcome.to_string(),
        diagnostic: attempt
            .diagnostic
            .as_deref()
            .map(sanitize_diagnostic)
            .filter(|value| !value.is_empty()),
        consecutive_failures,
        attempted_at: attempt.attempted_at.to_rfc3339(),
        next_retry_at: next_retry_at(attempt.attempted_at, consecutive_failures),
    }
}

fn consecutive_failure_count(
    outcome: MemoryCompactionOutcome,
    prior: Option<&MemoryCompactionAttemptRow>,
) -> i32 {
    if !outcome.is_failure() {
        return 0;
    }
    prior
        .filter(|attempt| stored_outcome_is_failure(attempt))
        .map_or(1, |attempt| attempt.consecutive_failures.saturating_add(1))
}

fn next_retry_at(attempted_at: DateTime<Utc>, consecutive_failures: i32) -> Option<String> {
    if consecutive_failures == 0 {
        return None;
    }
    let exponent = u32::try_from(consecutive_failures.saturating_sub(1))
        .unwrap_or(u32::MAX)
        .min(16);
    let seconds = 15_i64
        .saturating_mul(2_i64.saturating_pow(exponent))
        .min(3_600);
    Some((attempted_at + chrono::Duration::seconds(seconds)).to_rfc3339())
}

fn persist_attempt(
    connection: &mut SqliteConnection,
    job: &MemoryCompactionJob,
    attempt: &MemoryCompactionAttempt,
    prepared: &PreparedAttempt,
) -> Result<()> {
    diesel::insert_into(memory_compaction_attempts::table)
        .values(NewMemoryCompactionAttempt {
            id: &prepared.id,
            space_id: &job.space_id,
            level: i32::try_from(job.level)?,
            start_ordinal: i64::try_from(job.start_ordinal)?,
            end_ordinal: i64::try_from(job.end_ordinal)?,
            source_hash: &job.source_hash,
            provider: &attempt.provider,
            model_id: attempt.model_id.as_deref(),
            outcome: &prepared.outcome,
            diagnostic: prepared.diagnostic.as_deref(),
            was_fallback: attempt.was_fallback,
            consecutive_failures: prepared.consecutive_failures,
            attempted_at: &prepared.attempted_at,
            next_retry_at: prepared.next_retry_at.as_deref(),
        })
        .execute(connection)?;
    Ok(())
}

fn selected_spaces(
    connection: &mut SqliteConnection,
    project_id: Option<&str>,
) -> Result<Vec<MemorySpaceRow>> {
    let mut query = memory_spaces::table.into_boxed();
    if let Some(project_id) = project_id {
        query = query.filter(
            memory_spaces::scope
                .eq("global")
                .or(memory_spaces::project_id.eq(project_id)),
        );
    }
    query
        .order(memory_spaces::id.asc())
        .select(MemorySpaceRow::as_select())
        .load::<MemorySpaceRow>(connection)
        .map_err(Into::into)
}

fn load_health_snapshot(
    connection: &mut SqliteConnection,
    spaces: &[MemorySpaceRow],
    project_id: Option<&str>,
    now: DateTime<Utc>,
) -> Result<HealthSnapshot> {
    let scopes = load_scope_health_rows(connection, spaces, project_id)?;
    let space_ids = spaces
        .iter()
        .map(|space| space.id.clone())
        .collect::<Vec<_>>();
    let queue = load_queue_health(connection, &space_ids, now)?;
    let attempts = load_attempt_health(connection, &space_ids)?;
    Ok(HealthSnapshot {
        scopes,
        latest_attempts: attempts.latest_attempts,
        active_retries: queue.active_retries,
        pending_summaries: queue.pending_summaries,
        failed_attempts: attempts.failed_attempts,
        cooling_down_attempts: queue.cooling_down_attempts,
        fallback_uses: attempts.fallback_uses,
        degraded: queue.degraded,
        last_activation_at: last_activation_at(connection, project_id)?,
    })
}

fn load_queue_health(
    connection: &mut SqliteConnection,
    space_ids: &[String],
    now: DateTime<Utc>,
) -> Result<QueueHealthSnapshot> {
    Ok(QueueHealthSnapshot {
        pending_summaries: pending_summary_count(connection, space_ids)?,
        cooling_down_attempts: active_cooldown_count(connection, space_ids, now)?,
        active_retries: active_provider_retries(connection, space_ids, now)?,
        degraded: queued_dual_failure_exists(connection, space_ids)?,
    })
}

fn load_attempt_health(
    connection: &mut SqliteConnection,
    space_ids: &[String],
) -> Result<AttemptHealthSnapshot> {
    Ok(AttemptHealthSnapshot {
        latest_attempts: latest_provider_attempts(connection, space_ids)?,
        failed_attempts: failed_attempt_count(connection, space_ids)?,
        fallback_uses: fallback_use_count(connection, space_ids)?,
    })
}

fn load_scope_health_rows(
    connection: &mut SqliteConnection,
    spaces: &[MemorySpaceRow],
    project_id: Option<&str>,
) -> Result<Vec<MemoryScopeHealth>> {
    let mut rows = spaces
        .iter()
        .map(|space| load_scope_health(connection, space))
        .collect::<Result<Vec<_>>>()?;
    if let Some(project_id) = project_id {
        ensure_scope_health(&mut rows, MemoryScope::Global);
        ensure_scope_health(&mut rows, MemoryScope::project(project_id));
        rows.sort_by_key(|row| match row.scope {
            MemoryScope::Global => 0,
            MemoryScope::Project { .. } => 1,
        });
    } else if !rows
        .iter()
        .any(|row| matches!(row.scope, MemoryScope::Global))
    {
        rows.push(empty_scope_health(MemoryScope::Global));
    }
    Ok(rows)
}

fn ensure_scope_health(rows: &mut Vec<MemoryScopeHealth>, scope: MemoryScope) {
    if !rows.iter().any(|row| row.scope == scope) {
        rows.push(empty_scope_health(scope));
    }
}

fn empty_scope_health(scope: MemoryScope) -> MemoryScopeHealth {
    MemoryScopeHealth {
        scope,
        raw_entries: 0,
        summaries: 0,
    }
}

fn load_scope_health(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
) -> Result<MemoryScopeHealth> {
    let raw_entries = memory_entries::table
        .filter(memory_entries::space_id.eq(&space.id))
        .count()
        .get_result::<i64>(connection)?;
    let summaries = memory_summaries::table
        .filter(memory_summaries::space_id.eq(&space.id))
        .count()
        .get_result::<i64>(connection)?;
    Ok(MemoryScopeHealth {
        scope: super::compaction::scope_from_space(space)?,
        raw_entries,
        summaries,
    })
}

fn finalize_health(snapshot: HealthSnapshot) -> Result<MemoryHealth> {
    let providers = provider_health(&snapshot.latest_attempts, &snapshot.active_retries)?;
    Ok(MemoryHealth {
        scopes: snapshot.scopes,
        providers,
        pending_summaries: snapshot.pending_summaries,
        failed_attempts: snapshot.failed_attempts,
        cooling_down_attempts: snapshot.cooling_down_attempts,
        fallback_uses: snapshot.fallback_uses,
        degraded: snapshot.degraded,
        last_activation_at: snapshot.last_activation_at,
    })
}

fn pending_summary_count(connection: &mut SqliteConnection, space_ids: &[String]) -> Result<usize> {
    if space_ids.is_empty() {
        return Ok(0);
    }
    count_as_usize(
        memory_compaction_queue::table
            .filter(memory_compaction_queue::space_id.eq_any(space_ids))
            .count()
            .get_result(connection)?,
    )
}

fn failed_attempt_count(connection: &mut SqliteConnection, space_ids: &[String]) -> Result<usize> {
    if space_ids.is_empty() {
        return Ok(0);
    }
    count_as_usize(
        memory_compaction_attempts::table
            .filter(memory_compaction_attempts::space_id.eq_any(space_ids))
            .filter(memory_compaction_attempts::outcome.eq_any([
                "failed",
                "timed_out",
                "invalid_output",
            ]))
            .count()
            .get_result(connection)?,
    )
}

fn fallback_use_count(connection: &mut SqliteConnection, space_ids: &[String]) -> Result<usize> {
    if space_ids.is_empty() {
        return Ok(0);
    }
    count_as_usize(
        memory_compaction_attempts::table
            .filter(memory_compaction_attempts::space_id.eq_any(space_ids))
            .filter(memory_compaction_attempts::was_fallback.eq(true))
            .count()
            .get_result(connection)?,
    )
}

fn active_cooldown_count(
    connection: &mut SqliteConnection,
    space_ids: &[String],
    now: DateTime<Utc>,
) -> Result<usize> {
    if space_ids.is_empty() {
        return Ok(0);
    }
    let now = now.to_rfc3339();
    let codex = memory_compaction_queue::table
        .filter(memory_compaction_queue::space_id.eq_any(space_ids))
        .filter(memory_compaction_queue::codex_retry_at.gt(&now))
        .count()
        .get_result::<i64>(connection)?;
    let claude = memory_compaction_queue::table
        .filter(memory_compaction_queue::space_id.eq_any(space_ids))
        .filter(memory_compaction_queue::claude_retry_at.gt(&now))
        .count()
        .get_result::<i64>(connection)?;
    count_as_usize(
        codex
            .checked_add(claude)
            .context("memory cooldown count overflow")?,
    )
}

fn queued_dual_failure_exists(
    connection: &mut SqliteConnection,
    space_ids: &[String],
) -> Result<bool> {
    if space_ids.is_empty() {
        return Ok(false);
    }
    Ok(memory_compaction_queue::table
        .filter(memory_compaction_queue::space_id.eq_any(space_ids))
        .filter(memory_compaction_queue::codex_retry_at.is_not_null())
        .filter(memory_compaction_queue::claude_retry_at.is_not_null())
        .select(memory_compaction_queue::space_id)
        .first::<String>(connection)
        .optional()?
        .is_some())
}

fn latest_provider_attempts(
    connection: &mut SqliteConnection,
    space_ids: &[String],
) -> Result<Vec<MemoryCompactionAttemptRow>> {
    if space_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut attempts = Vec::new();
    for provider in ["claude", "codex"] {
        let attempt = memory_compaction_attempts::table
            .filter(memory_compaction_attempts::space_id.eq_any(space_ids))
            .filter(memory_compaction_attempts::provider.eq(provider))
            .order((
                memory_compaction_attempts::attempted_at.desc(),
                memory_compaction_attempts::id.desc(),
            ))
            .select(MemoryCompactionAttemptRow::as_select())
            .first::<MemoryCompactionAttemptRow>(connection)
            .optional()?;
        if let Some(attempt) = attempt {
            attempts.push(attempt);
        }
    }
    Ok(attempts)
}

fn active_provider_retries(
    connection: &mut SqliteConnection,
    space_ids: &[String],
    now: DateTime<Utc>,
) -> Result<BTreeMap<String, DateTime<Utc>>> {
    if space_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let now = now.to_rfc3339();
    let mut retries = BTreeMap::new();
    for provider in ["claude", "codex"] {
        if let Some(retry) = active_provider_retry(connection, space_ids, provider, &now)? {
            retries.insert(provider.to_owned(), retry);
        }
    }
    Ok(retries)
}

fn active_provider_retry(
    connection: &mut SqliteConnection,
    space_ids: &[String],
    provider: &str,
    now: &str,
) -> Result<Option<DateTime<Utc>>> {
    let stored = match provider {
        "codex" => memory_compaction_queue::table
            .filter(memory_compaction_queue::space_id.eq_any(space_ids))
            .filter(memory_compaction_queue::codex_retry_at.gt(now))
            .order(memory_compaction_queue::codex_retry_at.asc())
            .select(memory_compaction_queue::codex_retry_at)
            .first::<Option<String>>(connection)
            .optional()?,
        "claude" => memory_compaction_queue::table
            .filter(memory_compaction_queue::space_id.eq_any(space_ids))
            .filter(memory_compaction_queue::claude_retry_at.gt(now))
            .order(memory_compaction_queue::claude_retry_at.asc())
            .select(memory_compaction_queue::claude_retry_at)
            .first::<Option<String>>(connection)
            .optional()?,
        _ => bail!("unknown memory consolidation provider {provider}"),
    };
    stored
        .flatten()
        .map(|value| parse_datetime(&value))
        .transpose()
}

fn count_as_usize(value: i64) -> Result<usize> {
    Ok(usize::try_from(value)?)
}

fn stored_outcome_is_failure(attempt: &MemoryCompactionAttemptRow) -> bool {
    attempt
        .outcome
        .parse::<MemoryCompactionOutcome>()
        .is_ok_and(MemoryCompactionOutcome::is_failure)
}

fn provider_health(
    attempts: &[MemoryCompactionAttemptRow],
    active_retries: &BTreeMap<String, DateTime<Utc>>,
) -> Result<Vec<MemoryProviderHealth>> {
    attempts
        .iter()
        .map(|attempt| {
            let next_retry_at = active_retries.get(&attempt.provider).cloned();
            Ok(MemoryProviderHealth {
                provider: attempt.provider.clone(),
                model_id: attempt.model_id.clone(),
                last_outcome: attempt.outcome.parse().map_err(anyhow::Error::msg)?,
                last_attempted_at: parse_datetime(&attempt.attempted_at)?,
                next_retry_at,
                consecutive_failures: u32::try_from(attempt.consecutive_failures)?,
                cooling_down: next_retry_at.is_some(),
            })
        })
        .collect()
}

fn last_activation_at(
    connection: &mut SqliteConnection,
    project_id: Option<&str>,
) -> Result<Option<DateTime<Utc>>> {
    if let Some(project_id) = project_id {
        let global = latest_activation_for_scope(connection, None)?;
        let project = latest_activation_for_scope(connection, Some(project_id))?;
        return Ok(global.into_iter().chain(project).max());
    }
    memory_activations::table
        .order((
            memory_activations::activated_at.desc(),
            memory_activations::id.desc(),
        ))
        .select(memory_activations::activated_at)
        .first::<String>(connection)
        .optional()?
        .map(|value| parse_datetime(&value))
        .transpose()
}

fn latest_activation_for_scope(
    connection: &mut SqliteConnection,
    project_id: Option<&str>,
) -> Result<Option<DateTime<Utc>>> {
    let mut query = memory_activations::table.into_boxed();
    query = if let Some(project_id) = project_id {
        query.filter(memory_activations::project_id.eq(project_id))
    } else {
        query.filter(memory_activations::project_id.is_null())
    };
    query
        .order((
            memory_activations::activated_at.desc(),
            memory_activations::id.desc(),
        ))
        .select(memory_activations::activated_at)
        .first::<String>(connection)
        .optional()?
        .map(|value| parse_datetime(&value))
        .transpose()
}

pub(super) fn provider_can_attempt(
    connection: &mut SqliteConnection,
    job: &MemoryCompactionJob,
    provider: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let latest = latest_attempt(connection, job, provider)?;
    let Some(next_retry_at) = latest.and_then(|attempt| attempt.next_retry_at) else {
        return Ok(true);
    };
    Ok(parse_datetime(&next_retry_at)? <= now)
}

fn latest_attempt(
    connection: &mut SqliteConnection,
    job: &MemoryCompactionJob,
    provider: &str,
) -> Result<Option<MemoryCompactionAttemptRow>> {
    memory_compaction_attempts::table
        .filter(memory_compaction_attempts::space_id.eq(&job.space_id))
        .filter(memory_compaction_attempts::level.eq(i32::try_from(job.level)?))
        .filter(memory_compaction_attempts::start_ordinal.eq(i64::try_from(job.start_ordinal)?))
        .filter(memory_compaction_attempts::end_ordinal.eq(i64::try_from(job.end_ordinal)?))
        .filter(memory_compaction_attempts::source_hash.eq(&job.source_hash))
        .filter(memory_compaction_attempts::provider.eq(provider))
        .order(memory_compaction_attempts::attempted_at.desc())
        .select(MemoryCompactionAttemptRow::as_select())
        .first::<MemoryCompactionAttemptRow>(connection)
        .optional()
        .map_err(Into::into)
}

fn sanitize_diagnostic(diagnostic: &str) -> String {
    let without_controls = diagnostic
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut sanitized = without_controls
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    while sanitized.len() > 512 {
        sanitized.pop();
    }
    sanitized
}

fn parse_datetime(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}
