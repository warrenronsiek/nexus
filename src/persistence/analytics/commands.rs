// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
// @entrypoint Store::activity_commands
//! Per-command operation histograms for one activity category.
//!
//! Each bin is a typed `COUNT(...) FILTER (WHERE ...)` aggregate, so one grouped scan returns
//! a complete fixed grid on SQLite and PostgreSQL without backend-specific date arithmetic.
use super::queries::category_name;
use super::*;
use chrono::{DateTime, SubsecRound};
use diesel::dsl::{count, count_star};

type HalfGrid = (
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
);
type CommandRow = (String, i64, HalfGrid, HalfGrid);
type TotalRow = (i64, HalfGrid, HalfGrid);

// Two 14-bin halves keep each selected tuple inside Diesel's default 16-element support.
macro_rules! half_grid {
    ($edges:ident, $offset:literal) => {
        half_grid!(@bins $edges, $offset; 0 1 2 3 4 5 6 7 8 9 10 11 12 13)
    };
    (@bins $edges:ident, $offset:literal; $($bin:literal)+) => {
        ($(count(analytics_actions::id).aggregate_filter(
            analytics_actions::observed_at
                .ge($edges[$offset + $bin].as_str())
                .and(analytics_actions::observed_at.lt($edges[$offset + $bin + 1].as_str())),
        ),)+)
    };
}

macro_rules! category_operations {
    ($category:expr, $edges:ident) => {
        leaf_actions!()
            .filter(analytics_actions::category_id.eq($category))
            .filter(analytics_actions::observed_at.ge($edges[0].as_str()))
            .filter(analytics_actions::observed_at.lt($edges[COMMAND_BINS].as_str()))
    };
}

fn spread(grid: HalfGrid) -> [i64; 14] {
    let (a, b, c, d, e, f, g, h, i, j, k, l, m, n) = grid;
    [a, b, c, d, e, f, g, h, i, j, k, l, m, n]
}

fn bins(first: HalfGrid, second: HalfGrid) -> Vec<i64> {
    spread(first).into_iter().chain(spread(second)).collect()
}

impl Store {
    pub(crate) fn activity_commands(
        &mut self,
        project: Option<&str>,
        category: &str,
    ) -> Result<CommandHistogram> {
        self.activity_commands_at(project, category, Utc::now())
    }

    pub(super) fn activity_commands_at(
        &mut self,
        project: Option<&str>,
        category: &str,
        now: DateTime<Utc>,
    ) -> Result<CommandHistogram> {
        let edges = bin_edges(now);
        let mut named = self.command_rows(project, category, &edges)?;
        let truncated = named.len() > MAX_NAMED_COMMANDS;
        named.truncate(MAX_NAMED_COMMANDS);
        let mut commands: Vec<CommandSeries> = named
            .into_iter()
            .map(|(name, count, first, second)| series(name, count, bins(first, second), &edges))
            .collect();
        if truncated {
            let totals = self.command_totals(project, category, &edges)?;
            commands.push(other_commands(&commands, totals, &edges));
        }
        Ok(CommandHistogram {
            ok: true,
            category_id: category.into(),
            name: self.category_display_name(category)?,
            window_start: edges[0].clone(),
            window_end: edges[COMMAND_BINS].clone(),
            bin_seconds: COMMAND_BIN_SECONDS,
            commands,
            truncated,
            counting: COMMAND_COUNTING,
        })
    }

    fn command_rows(
        &mut self,
        project: Option<&str>,
        category: &str,
        edges: &[String],
    ) -> Result<Vec<CommandRow>> {
        let mut query = category_operations!(category, edges)
            .group_by(analytics_actions::name)
            .select((
                analytics_actions::name,
                count_star(),
                half_grid!(edges, 0),
                half_grid!(edges, 14),
            ))
            .order((count_star().desc(), analytics_actions::name.asc()))
            .limit(MAX_NAMED_COMMANDS as i64 + 1)
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            query = query.filter(analytics_actions::project_id.eq(project));
        }
        Ok(query.load(&mut self.connection)?)
    }

    fn command_totals(
        &mut self,
        project: Option<&str>,
        category: &str,
        edges: &[String],
    ) -> Result<TotalRow> {
        let mut query = category_operations!(category, edges)
            .select((count_star(), half_grid!(edges, 0), half_grid!(edges, 14)))
            .into_boxed::<DbBackend>();
        if let Some(project) = project {
            query = query.filter(analytics_actions::project_id.eq(project));
        }
        Ok(query.get_result(&mut self.connection)?)
    }

    fn category_display_name(&mut self, category: &str) -> Result<String> {
        Ok(analytics_categories::table
            .find(category)
            .select(analytics_categories::name)
            .first::<String>(&mut self.connection)
            .optional()?
            .unwrap_or_else(|| category_name(category)))
    }
}

const COMMAND_COUNTING: &str = "Counts are identified operations in this category, matching the summary's operation denominator. Bins are contiguous six-hour intervals starting at window_start; commands beyond the top 20 are folded into other.";

/// Whole-second edges read cleanly; rounding the end up keeps operations from this second.
fn bin_edges(now: DateTime<Utc>) -> Vec<String> {
    let whole = now.trunc_subsecs(0);
    let end = if whole == now {
        whole
    } else {
        whole + chrono::Duration::seconds(1)
    };
    let width = chrono::Duration::seconds(COMMAND_BIN_SECONDS);
    let start = end - width * COMMAND_BINS as i32;
    (0..=COMMAND_BINS as i32)
        .map(|bin| (start + width * bin).to_rfc3339())
        .collect()
}

/// Everything outside the named series, derived from category totals in each bin.
fn other_commands(named: &[CommandSeries], totals: TotalRow, edges: &[String]) -> CommandSeries {
    let (total, first, second) = totals;
    let mut remainder = bins(first, second);
    for command in named {
        for (left, bin) in remainder.iter_mut().zip(&command.bins) {
            *left -= bin.count;
        }
    }
    let named_total: i64 = named.iter().map(|command| command.count).sum();
    series(OTHER_COMMANDS.into(), total - named_total, remainder, edges)
}

fn series(name: String, count: i64, counts: Vec<i64>, edges: &[String]) -> CommandSeries {
    CommandSeries {
        name,
        count,
        bins: edges
            .iter()
            .zip(counts)
            .map(|(start, count)| CommandBin {
                start: start.clone(),
                count,
            })
            .collect(),
    }
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
