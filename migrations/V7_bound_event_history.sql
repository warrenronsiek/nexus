-- @feature persistence
-- @spec docs/features/persistence.md
UPDATE events
SET payload_json = '{"truncated":true,"reason":"oversized_historical_event_payload"}'
WHERE length(CAST(payload_json AS BLOB)) > 65536;

UPDATE events
SET session_id = 'oversized-legacy-session'
WHERE session_id IS NOT NULL
  AND length(CAST(session_id AS BLOB)) > 512;

CREATE TRIGGER events_payload_size_insert
BEFORE INSERT ON events
WHEN length(CAST(NEW.payload_json AS BLOB)) > 65536
BEGIN
    SELECT RAISE(ABORT, 'event payload exceeds 65536 bytes');
END;

CREATE TRIGGER events_payload_size_update
BEFORE UPDATE OF payload_json ON events
WHEN length(CAST(NEW.payload_json AS BLOB)) > 65536
BEGIN
    SELECT RAISE(ABORT, 'event payload exceeds 65536 bytes');
END;

CREATE TRIGGER events_session_size_insert
BEFORE INSERT ON events
WHEN NEW.session_id IS NOT NULL
 AND length(CAST(NEW.session_id AS BLOB)) > 512
BEGIN
    SELECT RAISE(ABORT, 'event session id exceeds 512 bytes');
END;

CREATE TRIGGER events_session_size_update
BEFORE UPDATE OF session_id ON events
WHEN NEW.session_id IS NOT NULL
 AND length(CAST(NEW.session_id AS BLOB)) > 512
BEGIN
    SELECT RAISE(ABORT, 'event session id exceeds 512 bytes');
END;
