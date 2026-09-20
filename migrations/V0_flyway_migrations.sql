-- @feature persistence
-- @spec docs/features/persistence.md
CREATE TABLE IF NOT EXISTS flyway_migrations (
    version INTEGER PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    checksum TEXT NOT NULL,
    status TEXT NOT NULL
);
