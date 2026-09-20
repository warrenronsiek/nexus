-- @feature persistence
-- @spec docs/features/persistence.md
CREATE INDEX advisories_session_created_at
ON advisories(session_id, created_at);
