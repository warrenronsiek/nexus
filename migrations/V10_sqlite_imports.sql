-- @feature persistence
-- @spec docs/features/persistence.md
CREATE TABLE database_imports (
    source_digest TEXT PRIMARY KEY NOT NULL CHECK(length(source_digest) = 64),
    source_kind TEXT NOT NULL CHECK(source_kind = 'sqlite'),
    format_version INTEGER NOT NULL CHECK(format_version > 0),
    row_count BIGINT NOT NULL CHECK(row_count >= 0),
    imported_at TEXT NOT NULL CHECK(length(imported_at) > 0)
);
