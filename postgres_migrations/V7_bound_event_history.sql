-- @feature persistence
-- @spec docs/features/persistence.md
UPDATE events
SET payload_json = '{"truncated":true,"reason":"oversized_historical_event_payload"}'
WHERE octet_length(payload_json) > 65536;

UPDATE events
SET session_id = 'oversized-legacy-session'
WHERE session_id IS NOT NULL
  AND octet_length(session_id) > 512;

ALTER TABLE events
ADD CONSTRAINT events_payload_size
CHECK (octet_length(payload_json) <= 65536);

ALTER TABLE events
ADD CONSTRAINT events_session_size
CHECK (session_id IS NULL OR octet_length(session_id) <= 512);
