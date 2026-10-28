// @feature persistence
// @feature work-history
// @spec docs/features/persistence.md
// @spec docs/features/work-history.md
use super::*;
use crate::persistence::schema::{work_links, work_observations, work_subjects};

#[test]
fn upgrades_v12_with_empty_work_observations_and_hierarchy_indexes() {
    let (_temp, path) = database_through("upgrade-v12.db", 12);
    assert_current_version(&path);
    let mut connection = establish_connection(path.to_str().unwrap()).unwrap();
    let counts = [
        work_observations::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        work_subjects::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
        work_links::table
            .count()
            .get_result::<i64>(&mut connection)
            .unwrap(),
    ];
    assert_eq!(counts, [0; 3]);
    let history = flyway_migrations::table
        .order(flyway_migrations::version.asc())
        .select((flyway_migrations::version, flyway_migrations::status))
        .load::<(i64, String)>(&mut connection)
        .unwrap();
    assert_eq!(
        history,
        (0..=CURRENT_SCHEMA_VERSION as i64)
            .map(|version| (version, "deployed".into()))
            .collect::<Vec<_>>()
    );
}
