// @feature agent-memory
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/persistence.md
use super::*;

impl Store {
    pub fn ready_memory_compactions(
        &mut self,
        providers: &[&str],
        now: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<MemoryCompactionJob>> {
        queue::ready_jobs(&mut self.connection, providers, now, limit)
    }

    pub fn memory_provider_can_attempt(
        &mut self,
        job: &MemoryCompactionJob,
        provider: &str,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        health::provider_can_attempt(&mut self.connection, job, provider, now)
    }

    pub fn insert_memory_summary(
        &mut self,
        job: &MemoryCompactionJob,
        content: &str,
        provider: &str,
        model_id: Option<&str>,
    ) -> Result<MemorySummaryInsert> {
        validate_compaction_job(job)?;
        if provider.trim().is_empty() {
            bail!("memory summary provider cannot be empty");
        }
        let pending = PendingSummary {
            content: normalize_memory_text(content, crate::memory::MAX_MEMORY_SUMMARY_BYTES)?,
            provider,
            model_id,
            now: Utc::now().to_rfc3339(),
        };
        self.connection
            .immediate_transaction::<_, anyhow::Error, _>(|connection| {
                insert_summary_transaction(connection, job, &pending)
            })
    }

    pub fn expand_memory_summary(&mut self, summary_id: &str) -> Result<Vec<MemoryNode>> {
        let row = load_summary_required(&mut self.connection, summary_id)?;
        let job = summary_expansion_job(&mut self.connection, row)?;
        let children = load_job_children(&mut self.connection, &job)?;
        if children.len() != 2 {
            bail!("memory summary {summary_id} has unavailable children");
        }
        Ok(children)
    }

    pub fn invalidate_memory_summary(&mut self, summary_id: &str) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        let (deleted, scope) = self
            .connection
            .immediate_transaction::<_, anyhow::Error, _>(|connection| {
                invalidate_summary_transaction(connection, summary_id, &now)
            })?;
        if deleted > 0 {
            self.repair_memory_frontier(&scope)?;
        }
        Ok(deleted)
    }
}

fn invalidate_summary_transaction(
    connection: &mut SqliteConnection,
    summary_id: &str,
    now: &str,
) -> Result<(usize, MemoryScope)> {
    let target = load_summary_required(connection, summary_id)?;
    let space = memory_spaces::table
        .find(&target.space_id)
        .select(MemorySpaceRow::as_select())
        .first::<MemorySpaceRow>(connection)?;
    let deleted = apply_summary_invalidation(connection, &target, now)?;
    Ok((deleted, scope_from_space(&space)?))
}

fn apply_summary_invalidation(
    connection: &mut SqliteConnection,
    target: &MemorySummaryRow,
    now: &str,
) -> Result<usize> {
    queue::prepare_summary_invalidation(connection, target)?;
    let deleted = delete_summary_and_ancestors(connection, target)?;
    if deleted > 0 {
        finish_summary_invalidation(connection, target, now)?;
    }
    Ok(deleted)
}

fn delete_summary_and_ancestors(
    connection: &mut SqliteConnection,
    target: &MemorySummaryRow,
) -> Result<usize> {
    diesel::delete(
        memory_summaries::table
            .filter(memory_summaries::space_id.eq(&target.space_id))
            .filter(memory_summaries::start_ordinal.le(target.start_ordinal))
            .filter(memory_summaries::end_ordinal.ge(target.end_ordinal)),
    )
    .execute(connection)
    .map_err(Into::into)
}

fn finish_summary_invalidation(
    connection: &mut SqliteConnection,
    target: &MemorySummaryRow,
    now: &str,
) -> Result<()> {
    queue::enqueue_invalidated_summary(connection, target, now)?;
    bump_space_revision(connection, &target.space_id, now)
}

struct PendingSummary<'a> {
    content: String,
    provider: &'a str,
    model_id: Option<&'a str>,
    now: String,
}

fn insert_summary_transaction(
    connection: &mut SqliteConnection,
    job: &MemoryCompactionJob,
    pending: &PendingSummary<'_>,
) -> Result<MemorySummaryInsert> {
    if let Some(existing) = existing_summary_insert(connection, job)? {
        return Ok(existing);
    }
    if !source_snapshot_matches(connection, job)? {
        return Ok(MemorySummaryInsert::Stale);
    }
    persist_summary(connection, job, pending)
}

fn existing_summary_insert(
    connection: &mut SqliteConnection,
    job: &MemoryCompactionJob,
) -> Result<Option<MemorySummaryInsert>> {
    let existing = find_summary_row(
        connection,
        &job.space_id,
        job.start_ordinal,
        job.end_ordinal,
    )?;
    let Some(existing) = existing else {
        return Ok(None);
    };
    if existing.source_hash != job.source_hash {
        return Ok(Some(MemorySummaryInsert::Stale));
    }
    Ok(Some(MemorySummaryInsert::AlreadyPresent(summary_from_row(
        existing,
        job.scope.clone(),
    )?)))
}

fn source_snapshot_matches(
    connection: &mut SqliteConnection,
    job: &MemoryCompactionJob,
) -> Result<bool> {
    let current_children = load_job_children(connection, job)?;
    Ok(current_children.len() == 2 && source_hash(&current_children) == job.source_hash)
}

fn persist_summary(
    connection: &mut SqliteConnection,
    job: &MemoryCompactionJob,
    pending: &PendingSummary<'_>,
) -> Result<MemorySummaryInsert> {
    let id = Uuid::new_v4().to_string();
    let content_hash = blake3::hash(pending.content.as_bytes())
        .to_hex()
        .to_string();
    insert_summary_row(connection, &id, &content_hash, job, pending)?;
    queue::complete_summary_insert(connection, job, &pending.now)?;
    bump_space_revision(connection, &job.space_id, &pending.now)?;
    let row = memory_summaries::table
        .find(id)
        .select(MemorySummaryRow::as_select())
        .first(connection)?;
    Ok(MemorySummaryInsert::Inserted(summary_from_row(
        row,
        job.scope.clone(),
    )?))
}

fn insert_summary_row(
    connection: &mut SqliteConnection,
    id: &str,
    content_hash: &str,
    job: &MemoryCompactionJob,
    pending: &PendingSummary<'_>,
) -> Result<()> {
    diesel::insert_into(memory_summaries::table)
        .values(NewMemorySummary {
            id,
            space_id: &job.space_id,
            level: i32::try_from(job.level)?,
            start_ordinal: i64::try_from(job.start_ordinal)?,
            end_ordinal: i64::try_from(job.end_ordinal)?,
            content: &pending.content,
            content_hash,
            source_hash: &job.source_hash,
            provider: pending.provider,
            model_id: pending.model_id,
            created_at: &pending.now,
        })
        .execute(connection)?;
    Ok(())
}

fn bump_space_revision(connection: &mut SqliteConnection, space_id: &str, now: &str) -> Result<()> {
    diesel::update(memory_spaces::table.find(space_id))
        .set((
            memory_spaces::revision.eq(memory_spaces::revision + 1),
            memory_spaces::updated_at.eq(now),
        ))
        .execute(connection)?;
    Ok(())
}

fn load_summary_required(
    connection: &mut SqliteConnection,
    summary_id: &str,
) -> Result<MemorySummaryRow> {
    memory_summaries::table
        .find(summary_id)
        .select(MemorySummaryRow::as_select())
        .first::<MemorySummaryRow>(connection)
        .optional()?
        .with_context(|| format!("memory summary {summary_id} does not exist"))
}

fn summary_expansion_job(
    connection: &mut SqliteConnection,
    row: MemorySummaryRow,
) -> Result<MemoryCompactionJob> {
    let space = memory_spaces::table
        .find(&row.space_id)
        .select(MemorySpaceRow::as_select())
        .first::<MemorySpaceRow>(connection)?;
    Ok(MemoryCompactionJob {
        job_id: String::new(),
        space_id: row.space_id,
        scope: scope_from_space(&space)?,
        level: u32::try_from(row.level)?,
        start_ordinal: u64::try_from(row.start_ordinal)?,
        end_ordinal: u64::try_from(row.end_ordinal)?,
        source_hash: row.source_hash,
        children: Vec::new(),
    })
}

pub(super) fn load_job_children(
    connection: &mut SqliteConnection,
    job: &MemoryCompactionJob,
) -> Result<Vec<MemoryNode>> {
    let half = (job.end_ordinal - job.start_ordinal) / 2;
    if job.level == 1 {
        return [job.start_ordinal, job.start_ordinal + 1]
            .into_iter()
            .map(|ordinal| {
                memory_entries::table
                    .filter(memory_entries::space_id.eq(&job.space_id))
                    .filter(memory_entries::ordinal.eq(i64::try_from(ordinal)?))
                    .select(MemoryEntryRow::as_select())
                    .first::<MemoryEntryRow>(connection)
                    .optional()?
                    .map(|row| entry_from_row(row, job.scope.clone()).map(MemoryNode::Raw))
                    .transpose()
            })
            .collect::<Result<Option<Vec<_>>>>()
            .map(|rows| rows.unwrap_or_default());
    }
    [
        (job.start_ordinal, job.start_ordinal + half),
        (job.start_ordinal + half, job.end_ordinal),
    ]
    .into_iter()
    .map(|(start, end)| {
        find_summary_row(connection, &job.space_id, start, end)?
            .map(|row| summary_from_row(row, job.scope.clone()).map(MemoryNode::Summary))
            .transpose()
    })
    .collect::<Result<Option<Vec<_>>>>()
    .map(|rows| rows.unwrap_or_default())
}

pub(super) fn source_hash(children: &[MemoryNode]) -> String {
    let mut hasher = blake3::Hasher::new();
    for child in children {
        match child {
            MemoryNode::Raw(entry) => {
                hasher.update(b"raw\0");
                hasher.update(&entry.ordinal.to_le_bytes());
                hasher.update(entry.content_hash.as_bytes());
            }
            MemoryNode::Summary(summary) => {
                hasher.update(b"summary\0");
                hasher.update(&summary.level.to_le_bytes());
                hasher.update(&summary.start_ordinal.to_le_bytes());
                hasher.update(&summary.end_ordinal.to_le_bytes());
                hasher.update(blake3::hash(summary.content.as_bytes()).as_bytes());
                hasher.update(summary.source_hash.as_bytes());
            }
        }
        hasher.update(b"\0");
    }
    hasher.finalize().to_hex().to_string()
}

pub(super) fn compaction_job_id(
    space_id: &str,
    level: u32,
    start: u64,
    end: u64,
    source_hash: &str,
) -> String {
    let identity = format!("{space_id}\0{level}\0{start}\0{end}\0{source_hash}");
    blake3::hash(identity.as_bytes()).to_hex().to_string()
}

pub(super) fn validate_compaction_job(job: &MemoryCompactionJob) -> Result<()> {
    let width = 1_u64
        .checked_shl(job.level)
        .context("memory compaction level is too large")?;
    let actual_width = job
        .end_ordinal
        .checked_sub(job.start_ordinal)
        .context("memory compaction range ends before it starts")?;
    if job.level == 0
        || actual_width != width
        || job.start_ordinal % width != 0
        || job.children.len() != 2
    {
        bail!("memory compaction job is not an aligned power-of-two range");
    }
    Ok(())
}

pub(super) fn find_summary_row(
    connection: &mut SqliteConnection,
    space_id: &str,
    start: u64,
    end: u64,
) -> Result<Option<MemorySummaryRow>> {
    memory_summaries::table
        .filter(memory_summaries::space_id.eq(space_id))
        .filter(memory_summaries::start_ordinal.eq(i64::try_from(start)?))
        .filter(memory_summaries::end_ordinal.eq(i64::try_from(end)?))
        .select(MemorySummaryRow::as_select())
        .first(connection)
        .optional()
        .map_err(Into::into)
}

pub(super) fn scope_from_space(space: &MemorySpaceRow) -> Result<MemoryScope> {
    match (space.scope.as_str(), space.project_id.as_deref()) {
        ("global", None) => Ok(MemoryScope::Global),
        ("project", Some(project_id)) if !project_id.is_empty() => {
            Ok(MemoryScope::project(project_id))
        }
        _ => bail!("memory space {} has an invalid scope", space.id),
    }
}

pub(super) fn summary_from_row(row: MemorySummaryRow, scope: MemoryScope) -> Result<MemorySummary> {
    Ok(MemorySummary {
        id: row.id,
        scope,
        level: u32::try_from(row.level).context("memory summary level is negative")?,
        start_ordinal: u64::try_from(row.start_ordinal)
            .context("memory summary start ordinal is negative")?,
        end_ordinal: u64::try_from(row.end_ordinal)
            .context("memory summary end ordinal is negative")?,
        content: row.content,
        source_hash: row.source_hash,
        provider: row.provider,
        model_id: row.model_id,
        created_at: DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
    })
}
