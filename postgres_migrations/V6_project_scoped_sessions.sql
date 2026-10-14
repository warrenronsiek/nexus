-- @feature persistence
-- @feature agent-memory
-- @spec docs/features/persistence.md
-- @spec docs/features/agent-memory.md
ALTER TABLE sessions DROP CONSTRAINT sessions_pkey;
ALTER TABLE sessions ADD PRIMARY KEY(project_id, session_id);

ALTER TABLE memory_activations ADD COLUMN activation_scope_key TEXT;
UPDATE memory_activations
SET activation_scope_key = read_scope || ':' || COALESCE(project_id, '');
ALTER TABLE memory_activations ALTER COLUMN activation_scope_key SET NOT NULL;
ALTER TABLE memory_activations
ADD CONSTRAINT memory_activations_scope_key_matches_scope
CHECK(activation_scope_key = read_scope || ':' || COALESCE(project_id, ''));
ALTER TABLE memory_activations
DROP CONSTRAINT memory_activations_agent_session_id_key;
ALTER TABLE memory_activations
ADD CONSTRAINT memory_activations_agent_session_scope_key
UNIQUE(agent, session_id, activation_scope_key);
