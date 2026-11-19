// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
// @boundary migration-fixture: simulate previously serialized delivery projections.
use super::*;
use crate::persistence::schema::{delivery_records, delivery_targets};

#[test]
fn upgrades_existing_contribution_heads_and_mappings_into_origin_indexes() {
    let (_temp, path) = database_through("delivery-revisions.db", 28);
    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    let head = "A".repeat(40);
    let mapping = "B".repeat(40);
    let payload = serde_json::json!({"body":{
        "contribution":{"head_revision":{"commit":head}},
        "mappings":[{"revision":{"commit":mapping}}]
    }});
    diesel::insert_into(delivery_records::table)
        .values((
            delivery_records::project_id.eq("project"),
            delivery_records::kind.eq("contribution"),
            delivery_records::id.eq("existing"),
            delivery_records::repository.eq("fixture/repo"),
            delivery_records::title.eq("Existing contribution"),
            delivery_records::state.eq("registered"),
            delivery_records::updated_at.eq("2026-11-01T00:00:00Z"),
            delivery_records::payload_json.eq(payload.to_string()),
        ))
        .execute(&mut connection)
        .unwrap();
    drop(connection);

    assert_current_version(&path);
    let mut connection = DbConnection::establish(path.to_str().unwrap()).unwrap();
    let revisions = delivery_targets::table
        .filter(delivery_targets::project_id.eq("project"))
        .filter(delivery_targets::subject_id.eq("existing"))
        .filter(delivery_targets::target_kind.eq("revision"))
        .order(delivery_targets::target.asc())
        .select(delivery_targets::target)
        .load::<String>(&mut connection)
        .unwrap();
    assert_eq!(revisions, ["a".repeat(40), "b".repeat(40)]);
}
