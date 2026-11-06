// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
use super::super::rows::AnalyticsActionsRow;
use super::*;
use chrono::{DateTime, Duration, TimeZone};

fn record(store: &mut Store, project: &str, category: &str, name: &str, at: DateTime<Utc>) {
    let id = uuid::Uuid::new_v4().to_string();
    diesel::insert_into(analytics_actions::table)
        .values(AnalyticsActionsRow {
            id: id.clone(),
            call_id: id,
            project_id: project.into(),
            session_key: "session".into(),
            resource_key: format!("cli:{name}"),
            name: name.into(),
            kind: "cli".into(),
            origin: "standard_cli".into(),
            category_id: category.into(),
            description: "Fixture operation".into(),
            pattern_key: "a".repeat(64),
            resource_version: None,
            evidence: "deterministic".into(),
            model_id: None,
            observed_at: at.to_rfc3339(),
        })
        .execute(&mut store.connection)
        .unwrap();
}

fn occupied(series: &CommandSeries) -> Vec<(usize, i64)> {
    series
        .bins
        .iter()
        .enumerate()
        .filter(|(_, bin)| bin.count > 0)
        .map(|(index, bin)| (index, bin.count))
        .collect()
}

fn totals(histogram: &CommandHistogram) -> Vec<(&str, i64)> {
    histogram
        .commands
        .iter()
        .map(|series| (series.name.as_str(), series.count))
        .collect()
}

fn window_end() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 8, 0, 0, 0).unwrap()
}

fn hours(value: i64) -> DateTime<Utc> {
    window_end() - Duration::days(7) + Duration::hours(value)
}

/// Read operations around the window edges, plus other categories and projects.
fn seeded_reads() -> Store {
    let mut store = Store::open_memory().unwrap();
    store.ensure_activity_categories().unwrap();
    let last = window_end() - Duration::nanoseconds(1);
    for at in [hours(0), hours(1), hours(5), last] {
        record(&mut store, "p", "files.read", "sed", at);
    }
    for at in [hours(6), hours(30)] {
        record(&mut store, "p", "files.read", "cat", at);
    }
    let outside = [hours(0) - Duration::nanoseconds(1), window_end()];
    for at in outside {
        record(&mut store, "p", "files.read", "head", at);
    }
    record(&mut store, "p", "code.search", "rg", hours(2));
    record(&mut store, "other", "files.read", "tail", hours(2));
    store
}

#[test]
fn command_histograms_describe_a_complete_rolling_grid() {
    let histogram = seeded_reads()
        .activity_commands_at(Some("p"), "files.read", window_end())
        .unwrap();

    assert_eq!(histogram.category_id, "files.read");
    assert_eq!(histogram.name, "files read");
    assert_eq!(histogram.window_start, hours(0).to_rfc3339());
    assert_eq!(histogram.window_end, window_end().to_rfc3339());
    assert_eq!(histogram.bin_seconds, 21_600);
    assert!(!histogram.truncated);
    for series in &histogram.commands {
        let starts: Vec<_> = series.bins.iter().map(|bin| bin.start.clone()).collect();
        let expected: Vec<_> = (0..28).map(|bin| hours(6 * bin).to_rfc3339()).collect();
        assert_eq!(starts, expected);
    }
}

#[test]
fn command_histograms_bin_only_the_selected_category_project_and_window() {
    let histogram = seeded_reads()
        .activity_commands_at(Some("p"), "files.read", window_end())
        .unwrap();

    assert_eq!(totals(&histogram), [("sed", 4), ("cat", 2)]);
    assert_eq!(occupied(&histogram.commands[0]), [(0, 3), (27, 1)]);
    assert_eq!(occupied(&histogram.commands[1]), [(1, 1), (5, 1)]);
}

#[test]
fn command_histograms_support_global_scope_and_empty_categories() {
    let mut store = seeded_reads();
    let global = store
        .activity_commands_at(None, "files.read", window_end())
        .unwrap();
    assert_eq!(totals(&global), [("sed", 4), ("cat", 2), ("tail", 1)]);
    let empty = store
        .activity_commands_at(Some("p"), "vcs.change", window_end())
        .unwrap();
    assert!(empty.commands.is_empty());
}

#[test]
fn operations_from_the_current_second_stay_inside_whole_second_edges() {
    let mut store = Store::open_memory().unwrap();
    let now = window_end() + Duration::milliseconds(250);
    record(
        &mut store,
        "p",
        "files.read",
        "cat",
        now - Duration::milliseconds(100),
    );

    let histogram = store
        .activity_commands_at(Some("p"), "files.read", now)
        .unwrap();

    let end = window_end() + Duration::seconds(1);
    assert_eq!(histogram.window_end, end.to_rfc3339());
    assert_eq!(
        histogram.window_start,
        (end - Duration::days(7)).to_rfc3339()
    );
    assert_eq!(totals(&histogram), [("cat", 1)]);
}

#[test]
fn command_histograms_count_operations_like_the_category_summary() {
    let mut store = Store::open_memory().unwrap();
    let end = window_end();
    let at = end - Duration::hours(1);
    record(&mut store, "p", "files.read", "cat", at);
    let parent = analytics_actions::table
        .select(analytics_actions::call_id)
        .first::<String>(&mut store.connection)
        .unwrap();
    // An orchestrating parent keeps its exact use but its children own the operation count.
    diesel::insert_into(analytics_calls::table)
        .values(AnalyticsCallsRow {
            id: "child".into(),
            project_id: "p".into(),
            agent: "pi".into(),
            session_key: "session".into(),
            invocation_id: "child".into(),
            parent_id: Some(parent),
            tool_name: "bash".into(),
            outcome: "succeeded".into(),
            observed_at: at.to_rfc3339(),
            completed_at: None,
        })
        .execute(&mut store.connection)
        .unwrap();
    record(&mut store, "p", "files.read", "tail", at);

    let histogram = store
        .activity_commands_at(Some("p"), "files.read", end)
        .unwrap();

    assert_eq!(totals(&histogram), [("tail", 1)]);
}

#[test]
fn command_histograms_fold_commands_beyond_the_named_limit_into_other() {
    let mut store = Store::open_memory().unwrap();
    let end = window_end();
    let start = end - Duration::days(7);
    for index in 0..23_i64 {
        // Earlier names are more frequent, so the folded tail is deterministic.
        for repeat in 0..(30 - index) {
            let at = start + Duration::hours(6 * (repeat % 2) + index % 6);
            let name = format!("tool-{index:02}");
            record(&mut store, "p", "unclassified", &name, at);
        }
    }

    let histogram = store
        .activity_commands_at(Some("p"), "unclassified", end)
        .unwrap();

    assert!(histogram.truncated);
    assert_eq!(histogram.commands.len(), 21);
    assert_eq!(histogram.commands[0].name, "tool-00");
    assert_eq!(histogram.commands[19].name, "tool-19");
    let other = &histogram.commands[20];
    assert_eq!(other.name, "other");
    assert_eq!(other.count, 10 + 9 + 8);
    assert_eq!(occupied(other), [(0, 5 + 5 + 4), (1, 5 + 4 + 4)]);
    assert_eq!(other.bins.len(), 28);
}
