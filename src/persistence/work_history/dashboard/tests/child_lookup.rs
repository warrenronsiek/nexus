// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
// @boundary disposable-query-plan: actual child-lookup work is measured using controlled fixture SELECTs only.
use super::*;
use crate::persistence::test_support::{postgres_query_plan, QueryPlanMode};
use diesel::connection::SimpleConnection;

#[test]
fn batched_child_existence_does_not_read_every_membership_edge() {
    let mut store = store();
    let project = uuid::Uuid::new_v4().to_string();
    record(
        &mut store,
        &project,
        ["root", "empty"]
            .iter()
            .map(|id| {
                observation(
                    id,
                    "session",
                    id,
                    json!({"event":"session_opened","title":id,"context":{}}),
                )
            })
            .collect(),
    );
    seed_fanout(&mut store, &project);
    store
        .connection
        .batch_execute("ANALYZE work_links; ANALYZE work_subjects;")
        .unwrap();
    let queries = capture_queries(&mut store.connection);
    let subjects: Vec<_> = ["root", "empty"]
        .iter()
        .map(|id| WorkSubject::Session { id: (*id).into() })
        .collect();
    let mut reader = projection::Projection::new(&mut store.connection, &project);
    reader.prefetch(&subjects).unwrap();
    let captured = queries
        .lock()
        .unwrap()
        .iter()
        .find(|sql| sql.contains("work_links"))
        .unwrap()
        .clone();
    assert!(reader.summary(&subjects[0]).unwrap().unwrap().has_children);
    assert!(!reader.summary(&subjects[1]).unwrap().unwrap().has_children);
    let plan = postgres_query_plan(&mut store.connection, &captured, QueryPlanMode::Analyzed);
    let visited = link_rows_visited(&plan);
    assert!(
        visited <= subjects.len() * 2,
        "child existence read {visited} membership rows for {} subjects:\n{plan}",
        subjects.len()
    );
}

fn link_rows_visited(plan: &str) -> usize {
    let rows = regex::Regex::new(r"rows=(\d+) loops=(\d+)").unwrap();
    plan.lines()
        .filter(|line| line.contains(" on work_links"))
        .map(|line| {
            let values = rows.captures(line).expect("analyzed membership scan");
            values[1].parse::<usize>().unwrap() * values[2].parse::<usize>().unwrap()
        })
        .sum()
}

fn seed_fanout(store: &mut Store, project: &str) {
    for start in (0..1200).step_by(32) {
        let events = (start..(start + 32).min(1200)).map(|i| {
            let id = format!("child-{i}");
            let root = if i < 1000 {"root".into()} else {format!("unopened-{i}")};
            observation(&id, "activity", &id, json!({"event":"activity_declared","name":id,"kind":"tool","work_session_id":root,"context":{}}))
        }).collect();
        record(store, project, events);
    }
}
