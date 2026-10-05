// @feature agent-memory
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/persistence.md
use super::*;

#[derive(Clone, Copy)]
enum ProviderSelection {
    Any,
    Codex,
    Claude,
    Either,
}

#[derive(Clone, Copy)]
struct QueueRange {
    level: u32,
    start: u64,
    end: u64,
}

pub(super) fn enqueue_raw_parent(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
    entry: &MemoryEntry,
    now: &str,
) -> Result<()> {
    if entry.ordinal % 2 == 0 {
        return Ok(());
    }
    let range = QueueRange {
        level: 1,
        start: entry.ordinal - 1,
        end: entry.ordinal + 1,
    };
    enqueue_range_if_ready(connection, space, range, now)?;
    Ok(())
}

pub(super) fn complete_summary_insert(
    connection: &mut SqliteConnection,
    job: &MemoryCompactionJob,
    now: &str,
) -> Result<()> {
    delete_queue_range(
        connection,
        &job.space_id,
        job.level,
        job.start_ordinal,
        job.end_ordinal,
    )?;
    let Some(parent) = parent_range(job) else {
        return Ok(());
    };
    let space = memory_spaces::table
        .find(&job.space_id)
        .select(MemorySpaceRow::as_select())
        .first::<MemorySpaceRow>(connection)?;
    enqueue_range_if_ready(connection, &space, parent, now)?;
    Ok(())
}

fn parent_range(job: &MemoryCompactionJob) -> Option<QueueRange> {
    let level = job.level.checked_add(1)?;
    let width = job.end_ordinal.checked_sub(job.start_ordinal)?;
    let parent_width = width.checked_mul(2)?;
    let start = job.start_ordinal / parent_width * parent_width;
    let end = start.checked_add(parent_width)?;
    Some(QueueRange { level, start, end })
}

pub(super) fn prepare_summary_invalidation(
    connection: &mut SqliteConnection,
    target: &MemorySummaryRow,
) -> Result<()> {
    diesel::delete(
        memory_compaction_queue::table
            .filter(memory_compaction_queue::space_id.eq(&target.space_id))
            .filter(memory_compaction_queue::start_ordinal.le(target.start_ordinal))
            .filter(memory_compaction_queue::end_ordinal.ge(target.end_ordinal)),
    )
    .execute(connection)?;
    Ok(())
}

pub(super) fn enqueue_invalidated_summary(
    connection: &mut SqliteConnection,
    target: &MemorySummaryRow,
    now: &str,
) -> Result<()> {
    let space = memory_spaces::table
        .find(&target.space_id)
        .select(MemorySpaceRow::as_select())
        .first::<MemorySpaceRow>(connection)?;
    let range = QueueRange {
        level: u32::try_from(target.level)?,
        start: u64::try_from(target.start_ordinal)?,
        end: u64::try_from(target.end_ordinal)?,
    };
    enqueue_range_if_ready(connection, &space, range, now)?;
    Ok(())
}

pub(super) fn ready_jobs(
    connection: &mut SqliteConnection,
    providers: &[&str],
    now: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<MemoryCompactionJob>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    load_ready_jobs(connection, providers, now, limit)
}

fn load_ready_jobs(
    connection: &mut SqliteConnection,
    providers: &[&str],
    now: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<MemoryCompactionJob>> {
    let selection = provider_selection(providers)?;
    let now_text = now.to_rfc3339();
    let Some(level) = first_ready_level(connection, selection, &now_text)? else {
        return Ok(Vec::new());
    };
    let rows = load_ready_rows(
        connection,
        selection,
        &now_text,
        level,
        i64::try_from(limit.min(8))?,
    )?;
    materialize_jobs(connection, rows)
}

pub(super) fn update_provider_retry(
    connection: &mut SqliteConnection,
    job: &MemoryCompactionJob,
    provider: &str,
    next_retry_at: Option<&str>,
) -> Result<()> {
    let target = memory_compaction_queue::table
        .filter(memory_compaction_queue::space_id.eq(&job.space_id))
        .filter(memory_compaction_queue::level.eq(i32::try_from(job.level)?))
        .filter(memory_compaction_queue::start_ordinal.eq(i64::try_from(job.start_ordinal)?))
        .filter(memory_compaction_queue::end_ordinal.eq(i64::try_from(job.end_ordinal)?))
        .filter(memory_compaction_queue::source_hash.eq(&job.source_hash));
    match provider {
        "codex" => diesel::update(target)
            .set(memory_compaction_queue::codex_retry_at.eq(next_retry_at))
            .execute(connection)?,
        "claude" => diesel::update(target)
            .set(memory_compaction_queue::claude_retry_at.eq(next_retry_at))
            .execute(connection)?,
        _ => bail!("unknown memory consolidation provider {provider}"),
    };
    Ok(())
}

fn provider_selection(providers: &[&str]) -> Result<ProviderSelection> {
    let codex = providers.contains(&"codex");
    let claude = providers.contains(&"claude");
    if providers
        .iter()
        .any(|provider| !matches!(*provider, "codex" | "claude"))
    {
        bail!("unknown memory consolidation provider");
    }
    Ok(match (providers.is_empty(), codex, claude) {
        (true, _, _) => ProviderSelection::Any,
        (_, true, true) => ProviderSelection::Either,
        (_, true, false) => ProviderSelection::Codex,
        (_, false, true) => ProviderSelection::Claude,
        _ => ProviderSelection::Any,
    })
}

fn first_ready_level(
    connection: &mut SqliteConnection,
    selection: ProviderSelection,
    now: &str,
) -> Result<Option<i32>> {
    ready_query(selection, now)
        .order((
            memory_compaction_queue::level.asc(),
            memory_compaction_queue::space_id.asc(),
            memory_compaction_queue::start_ordinal.asc(),
        ))
        .select(memory_compaction_queue::level)
        .first(connection)
        .optional()
        .map_err(Into::into)
}

fn load_ready_rows(
    connection: &mut SqliteConnection,
    selection: ProviderSelection,
    now: &str,
    level: i32,
    limit: i64,
) -> Result<Vec<MemoryCompactionQueueRow>> {
    ready_query(selection, now)
        .filter(memory_compaction_queue::level.eq(level))
        .order((
            memory_compaction_queue::space_id.asc(),
            memory_compaction_queue::start_ordinal.asc(),
        ))
        .limit(limit)
        .select(MemoryCompactionQueueRow::as_select())
        .load(connection)
        .map_err(Into::into)
}

fn ready_query<'a>(
    selection: ProviderSelection,
    now: &'a str,
) -> memory_compaction_queue::BoxedQuery<'a, diesel::sqlite::Sqlite> {
    let query = memory_compaction_queue::table.into_boxed();
    match selection {
        ProviderSelection::Any => query,
        ProviderSelection::Codex => query.filter(
            memory_compaction_queue::codex_retry_at
                .is_null()
                .or(memory_compaction_queue::codex_retry_at.le(now)),
        ),
        ProviderSelection::Claude => query.filter(
            memory_compaction_queue::claude_retry_at
                .is_null()
                .or(memory_compaction_queue::claude_retry_at.le(now)),
        ),
        ProviderSelection::Either => query.filter(
            memory_compaction_queue::codex_retry_at
                .is_null()
                .or(memory_compaction_queue::codex_retry_at.le(now))
                .or(memory_compaction_queue::claude_retry_at.is_null())
                .or(memory_compaction_queue::claude_retry_at.le(now)),
        ),
    }
}

fn materialize_jobs(
    connection: &mut SqliteConnection,
    rows: Vec<MemoryCompactionQueueRow>,
) -> Result<Vec<MemoryCompactionJob>> {
    let mut jobs = Vec::new();
    for row in rows {
        let space = memory_spaces::table
            .find(&row.space_id)
            .select(MemorySpaceRow::as_select())
            .first::<MemorySpaceRow>(connection)?;
        let mut job = queue_job(&space, row)?;
        let children = compaction::load_job_children(connection, &job)?;
        if children.len() != 2 || compaction::source_hash(&children) != job.source_hash {
            delete_queue_range(
                connection,
                &job.space_id,
                job.level,
                job.start_ordinal,
                job.end_ordinal,
            )?;
            let range = QueueRange {
                level: job.level,
                start: job.start_ordinal,
                end: job.end_ordinal,
            };
            enqueue_range_if_ready(connection, &space, range, &Utc::now().to_rfc3339())?;
            continue;
        }
        job.children = children;
        jobs.push(job);
    }
    Ok(jobs)
}

fn queue_job(space: &MemorySpaceRow, row: MemoryCompactionQueueRow) -> Result<MemoryCompactionJob> {
    let level = u32::try_from(row.level)?;
    let start_ordinal = u64::try_from(row.start_ordinal)?;
    let end_ordinal = u64::try_from(row.end_ordinal)?;
    Ok(MemoryCompactionJob {
        job_id: compaction::compaction_job_id(
            &row.space_id,
            level,
            start_ordinal,
            end_ordinal,
            &row.source_hash,
        ),
        space_id: row.space_id,
        scope: compaction::scope_from_space(space)?,
        level,
        start_ordinal,
        end_ordinal,
        source_hash: row.source_hash,
        children: Vec::new(),
    })
}

fn enqueue_range_if_ready(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
    range: QueueRange,
    now: &str,
) -> Result<bool> {
    let Some(source_hash) = queue_source(connection, space, range)? else {
        return Ok(false);
    };
    persist_queue_range(connection, space, range, &source_hash, now)?;
    Ok(true)
}

fn queue_source(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
    range: QueueRange,
) -> Result<Option<String>> {
    if compaction::find_summary_row(connection, &space.id, range.start, range.end)?.is_some() {
        return Ok(None);
    }
    let scope = compaction::scope_from_space(space)?;
    let placeholder = MemoryCompactionJob {
        job_id: String::new(),
        space_id: space.id.clone(),
        scope,
        level: range.level,
        start_ordinal: range.start,
        end_ordinal: range.end,
        source_hash: String::new(),
        children: Vec::new(),
    };
    let children = compaction::load_job_children(connection, &placeholder)?;
    Ok((children.len() == 2).then(|| compaction::source_hash(&children)))
}

fn persist_queue_range(
    connection: &mut SqliteConnection,
    space: &MemorySpaceRow,
    range: QueueRange,
    source_hash: &str,
    now: &str,
) -> Result<()> {
    let row = NewMemoryCompactionQueue {
        space_id: &space.id,
        level: i32::try_from(range.level)?,
        start_ordinal: i64::try_from(range.start)?,
        end_ordinal: i64::try_from(range.end)?,
        source_hash,
        codex_retry_at: None,
        claude_retry_at: None,
        queued_at: now,
    };
    diesel::insert_into(memory_compaction_queue::table)
        .values(&row)
        .on_conflict((
            memory_compaction_queue::space_id,
            memory_compaction_queue::level,
            memory_compaction_queue::start_ordinal,
            memory_compaction_queue::end_ordinal,
        ))
        .do_update()
        .set((
            memory_compaction_queue::source_hash.eq(row.source_hash),
            memory_compaction_queue::codex_retry_at.eq(row.codex_retry_at),
            memory_compaction_queue::claude_retry_at.eq(row.claude_retry_at),
            memory_compaction_queue::queued_at.eq(row.queued_at),
        ))
        .execute(connection)?;
    Ok(())
}

fn delete_queue_range(
    connection: &mut SqliteConnection,
    space_id: &str,
    level: u32,
    start: u64,
    end: u64,
) -> Result<()> {
    diesel::delete(
        memory_compaction_queue::table
            .filter(memory_compaction_queue::space_id.eq(space_id))
            .filter(memory_compaction_queue::level.eq(i32::try_from(level)?))
            .filter(memory_compaction_queue::start_ordinal.eq(i64::try_from(start)?))
            .filter(memory_compaction_queue::end_ordinal.eq(i64::try_from(end)?)),
    )
    .execute(connection)?;
    Ok(())
}
