// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::*;
fn populate_display_rows(
    projection: &mut DashboardProjection,
    start: chrono::DateTime<chrono::Utc>,
) {
    for index in 0..3000 {
        let mut item = WorkSummary::unobserved(&WorkSubject::Activity {
            id: format!("task-{index}"),
        });
        item.title = "t".repeat(512);
        item.kind = format!("{index:04}{}", "k".repeat(508));
        item.context
            .labels
            .insert("metadata".into(), "v".repeat(1024));
        projection
            .view
            .intervals
            .push(WorkIntervalNode::from(&item));
        projection.view.activities.push(item.clone());
        projection
            .view
            .activity_kinds
            .push(ActivityDurationStatistics {
                kind: item.kind.clone(),
                accounting_role: Some("execution".into()),
                duration: DurationStatistics::default(),
            });
        projection
            .view
            .activity_daily
            .push(DailyActivityStatistics {
                date: "2026-10-27".into(),
                kind: item.kind.clone(),
                duration: DurationStatistics::default(),
            });
        if index < 512 {
            item.subject = WorkSubject::Session {
                id: format!("session-{index}"),
            };
            projection.view.sessions.push(DashboardSession {
                summary: item,
                request_count: 4,
                timed_request_count: 4,
                active_seconds: Some(1.0),
            });
            projection.view.daily.push(DailyDurationStatistics {
                date: (start.date_naive() + chrono::Duration::days(index)).to_string(),
                session_count: 1,
                known_count: 1,
                total_seconds: 1.0,
                average_seconds: Some(1.0),
                active_known_count: 1,
                active_total_seconds: 1.0,
                active_average_seconds: Some(1.0),
            });
        }
    }
}
#[test]
fn bounded_dashboard_with_escaped_identities_fits_the_remote_response_budget() {
    let mut root = WorkSummary::unobserved(&WorkSubject::Session { id: "root".into() });
    root.work_session_ids = (0..64)
        .map(|index| format!("{index:02}{}", "\u{0001}".repeat(510)))
        .collect();
    let mut projection = DashboardProjection::new(Some("root"), false, WorkSessionOrigin::All);
    projection.view.selected_session = Some(root.clone());
    let start: chrono::DateTime<chrono::Utc> = "2026-10-27T00:00:00Z".parse().unwrap();
    populate_display_rows(&mut projection, start);
    let mut accounting = session_accounting(&root, &[], false);
    accounting.unobserved_intervals = (0..3000)
        .map(|index| AccountingInterval {
            started_at: start + chrono::Duration::seconds(index * 2),
            finished_at: start + chrono::Duration::seconds(index * 2 + 1),
        })
        .collect();
    accounting.coverage.observed_adapters = vec!["adapter".repeat(73); 32];
    projection.view.accounting = Some(accounting);
    projection.bound_output().unwrap();
    let bytes = serde_json::to_vec(&projection.view).unwrap().len();
    assert!(
        bytes <= 896 * 1024,
        "encoded dashboard size {bytes} needs headroom inside the 1MiB remote envelope"
    );
    assert!(projection.view.accounting.unwrap().coverage.display_partial);
}
