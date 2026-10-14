-- @feature persistence
-- @spec docs/features/persistence.md
CREATE TABLE database_imports (
    source_digest TEXT PRIMARY KEY NOT NULL CHECK(length(source_digest) = 64),
    source_kind TEXT NOT NULL CHECK(source_kind = 'sqlite'),
    format_version INTEGER NOT NULL CHECK(format_version > 0),
    row_count BIGINT NOT NULL CHECK(row_count >= 0),
    imported_at TEXT NOT NULL CHECK(length(imported_at) > 0)
);

-- Explicit event IDs are part of a portable snapshot. Keep PostgreSQL's identity
-- generator ahead of the imported history so the next ordinary event is unique.
CREATE FUNCTION nexus_sync_event_identity(imported_max BIGINT)
RETURNS BIGINT
LANGUAGE plpgsql
AS $$
DECLARE
    sequence_name REGCLASS;
    current_max BIGINT;
BEGIN
    SELECT pg_get_serial_sequence('events', 'id')::REGCLASS INTO sequence_name;
    SELECT COALESCE(MAX(id), 0) INTO current_max FROM events;
    IF GREATEST(imported_max, current_max) > 0 THEN
        PERFORM setval(sequence_name, GREATEST(imported_max, current_max), TRUE);
        RETURN GREATEST(imported_max, current_max);
    END IF;
    RETURN 0;
END;
$$;
