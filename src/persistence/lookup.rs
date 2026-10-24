// @feature persistence
// @spec docs/features/persistence.md
use super::backend::DbConnection;
use diesel::query_dsl::methods::FindDsl;
use diesel::query_dsl::LoadQuery;
use diesel::{OptionalExtension, QueryResult, RunQueryDsl};

/// Load a row by its complete primary key, preserving absence as an ordinary result.
pub(super) fn optional<'query, Row, Table, Key>(
    connection: &mut DbConnection,
    table: Table,
    key: Key,
) -> QueryResult<Option<Row>>
where
    Table: FindDsl<Key>,
    diesel::dsl::Find<Table, Key>: LoadQuery<'query, DbConnection, Row>,
{
    table.find(key).get_result(connection).optional()
}
