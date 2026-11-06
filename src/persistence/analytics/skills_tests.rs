// @feature persistence
// @feature usage-analytics
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
use super::*;

fn document(content: &str) -> SkillDocument {
    SkillDocument {
        content: content.into(),
        source_path: ".codex/skills/tdd/SKILL.md".into(),
    }
}

fn stored_observed_at(store: &mut Store, project: &str) -> String {
    analytics_skill_documents::table
        .filter(analytics_skill_documents::project_id.eq(project))
        .select(analytics_skill_documents::observed_at)
        .first(&mut store.connection)
        .unwrap()
}

#[test]
fn the_latest_skill_text_is_kept_per_project_and_unchanged_text_is_not_rewritten() {
    let mut store = Store::open_memory().unwrap();
    let first = "---\nname: tdd\ndescription: Test first.\n---\n# TDD\n";
    store
        .record_skill_document("p", "tdd", &document(first))
        .unwrap();
    let observed = stored_observed_at(&mut store, "p");
    store
        .record_skill_document("p", "tdd", &document(first))
        .unwrap();
    assert_eq!(stored_observed_at(&mut store, "p"), observed);

    let detail = store.skill_detail(Some("p"), "tdd").unwrap();
    assert_eq!(detail.name, "tdd");
    assert_eq!(detail.description.as_deref(), Some("Test first."));
    assert_eq!(detail.content.as_deref(), Some(first));
    assert_eq!(detail.content_hash, Some(document(first).content_hash()));
    assert_eq!(
        detail.source_path.as_deref(),
        Some(".codex/skills/tdd/SKILL.md")
    );
    assert_eq!(detail.observed_at.as_deref(), Some(observed.as_str()));

    let revised = "---\nname: tdd\n---\n# TDD v2\n";
    store
        .record_skill_document("p", "tdd", &document(revised))
        .unwrap();
    let detail = store.skill_detail(Some("p"), "tdd").unwrap();
    assert_eq!(detail.content.as_deref(), Some(revised));
    assert_eq!(detail.description, None);
    let rows = analytics_skill_documents::table
        .count()
        .get_result::<i64>(&mut store.connection)
        .unwrap();
    assert_eq!(rows, 1);

    store
        .record_skill_document("q", "tdd", &document("# Q\n"))
        .unwrap();
    assert_eq!(
        store
            .skill_detail(Some("p"), "tdd")
            .unwrap()
            .content
            .as_deref(),
        Some(revised)
    );
    assert!(store.skill_detail(None, "tdd").unwrap().content.is_some());
}

#[test]
fn a_skill_without_captured_text_reads_as_null_fields() {
    let mut store = Store::open_memory().unwrap();
    let detail = store
        .skill_detail(Some("p"), "observed-by-an-older-client")
        .unwrap();
    assert_eq!(detail.name, "observed-by-an-older-client");
    assert!(detail.content.is_none() && detail.content_hash.is_none());
    assert!(detail.description.is_none() && detail.source_path.is_none());
    assert!(detail.observed_at.is_none());
}

#[test]
fn an_inventoried_skill_description_is_used_when_no_text_was_captured() {
    let mut store = Store::open_memory().unwrap();
    let context: crate::coordination::domain::HookContext =
        serde_json::from_value(serde_json::json!({"session_id":"s","agent":"pi"})).unwrap();
    let skill = Resource {
        name: "review".into(),
        kind: ResourceKind::Skill,
        origin: Origin::HostTool,
        exposure: Exposure::Exposed,
        description: "Review staged changes".into(),
        activities: vec![],
        version: None,
    };
    store
        .record_inventory("p", &context, "host_skills", &[skill], true)
        .unwrap();
    let detail = store.skill_detail(Some("p"), "review").unwrap();
    assert_eq!(detail.description.as_deref(), Some("Review staged changes"));
    assert!(detail.content.is_none());
}
