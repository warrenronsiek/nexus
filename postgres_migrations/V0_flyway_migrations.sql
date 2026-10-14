-- @feature persistence
-- @spec docs/features/persistence.md
CREATE TABLE IF NOT EXISTS flyway_migrations (
    version BIGINT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    checksum TEXT NOT NULL,
    status TEXT NOT NULL
);
