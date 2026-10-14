-- @feature persistence
-- @feature coordination
-- @feature agent-memory
-- @feature papercuts
-- @spec docs/features/persistence.md
-- @spec docs/features/coordination.md
-- @spec docs/features/agent-memory.md
-- @spec docs/features/papercuts.md

-- Preserve one bounded, shared representation for every oversized path before
-- enforcing the projection limits. The suffix keeps distinct legacy paths
-- distinct when they share the same 4 KiB prefix.
CREATE TEMP TABLE nexus_v8_path_map (
    mapping_id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL,
    old_path TEXT NOT NULL,
    new_path TEXT,
    UNIQUE(project_id, old_path)
);

INSERT OR IGNORE INTO nexus_v8_path_map(project_id, old_path)
SELECT project_id, path FROM claims
WHERE length(CAST(path AS BLOB)) > 4096
ORDER BY project_id, path;

INSERT OR IGNORE INTO nexus_v8_path_map(project_id, old_path)
SELECT project_id, path FROM conflicts
WHERE length(CAST(path AS BLOB)) > 4096
ORDER BY project_id, path;

INSERT OR IGNORE INTO nexus_v8_path_map(project_id, old_path)
SELECT project_id, path FROM advisories
WHERE length(CAST(path AS BLOB)) > 4096
ORDER BY project_id, path;

UPDATE nexus_v8_path_map
SET new_path = (
    WITH RECURSIVE candidates(attempt, candidate) AS (
        SELECT 0, (
            WITH RECURSIVE bounds(low, high) AS (
                VALUES(0, MIN(length(nexus_v8_path_map.old_path), 4070))
                UNION ALL
                SELECT
                    CASE WHEN length(CAST(substr(
                        nexus_v8_path_map.old_path,
                        1,
                        (low + high + 1) / 2
                    ) AS BLOB)) <= 4070
                    THEN (low + high + 1) / 2 ELSE low END,
                    CASE WHEN length(CAST(substr(
                        nexus_v8_path_map.old_path,
                        1,
                        (low + high + 1) / 2
                    ) AS BLOB)) <= 4070
                    THEN high ELSE ((low + high + 1) / 2) - 1 END
                FROM bounds WHERE low < high
            )
            SELECT substr(nexus_v8_path_map.old_path, 1, MAX(low)) FROM bounds
        ) || '~nexus-v8-' || printf('%016x', nexus_v8_path_map.mapping_id)
        UNION ALL
        SELECT attempt + 1, (
            WITH RECURSIVE bounds(low, high) AS (
                VALUES(0, MIN(length(nexus_v8_path_map.old_path), 4053))
                UNION ALL
                SELECT
                    CASE WHEN length(CAST(substr(
                        nexus_v8_path_map.old_path,
                        1,
                        (low + high + 1) / 2
                    ) AS BLOB)) <= 4053
                    THEN (low + high + 1) / 2 ELSE low END,
                    CASE WHEN length(CAST(substr(
                        nexus_v8_path_map.old_path,
                        1,
                        (low + high + 1) / 2
                    ) AS BLOB)) <= 4053
                    THEN high ELSE ((low + high + 1) / 2) - 1 END
                FROM bounds WHERE low < high
            )
            SELECT substr(nexus_v8_path_map.old_path, 1, MAX(low)) FROM bounds
        ) || '~nexus-v8-' || printf('%016x', nexus_v8_path_map.mapping_id)
          || '-' || printf('%016x', attempt + 1)
        FROM candidates
        WHERE EXISTS (
            SELECT 1 FROM claims
            WHERE claims.project_id = nexus_v8_path_map.project_id
              AND claims.path = candidates.candidate
            UNION ALL
            SELECT 1 FROM conflicts
            WHERE conflicts.project_id = nexus_v8_path_map.project_id
              AND conflicts.path = candidates.candidate
            UNION ALL
            SELECT 1 FROM advisories
            WHERE advisories.project_id = nexus_v8_path_map.project_id
              AND advisories.path = candidates.candidate
            UNION ALL
            SELECT 1 FROM nexus_v8_path_map AS assigned
            WHERE assigned.mapping_id <> nexus_v8_path_map.mapping_id
              AND assigned.project_id = nexus_v8_path_map.project_id
              AND assigned.new_path = candidates.candidate
        )
    )
    SELECT candidate FROM candidates ORDER BY attempt DESC LIMIT 1
);

UPDATE claims
SET path = (
    SELECT new_path FROM nexus_v8_path_map
    WHERE nexus_v8_path_map.project_id = claims.project_id
      AND nexus_v8_path_map.old_path = claims.path
)
WHERE length(CAST(path AS BLOB)) > 4096;

UPDATE conflicts
SET path = (
    SELECT new_path FROM nexus_v8_path_map
    WHERE nexus_v8_path_map.project_id = conflicts.project_id
      AND nexus_v8_path_map.old_path = conflicts.path
)
WHERE length(CAST(path AS BLOB)) > 4096;

UPDATE advisories
SET path = (
    SELECT new_path FROM nexus_v8_path_map
    WHERE nexus_v8_path_map.project_id = advisories.project_id
      AND nexus_v8_path_map.old_path = advisories.path
)
WHERE length(CAST(path AS BLOB)) > 4096;

-- Claim IDs are referenced by conflict projections. Map both sides together so
-- an upgrade cannot orphan an otherwise valid conflict.
CREATE TEMP TABLE nexus_v8_claim_id_map (
    mapping_id INTEGER PRIMARY KEY AUTOINCREMENT,
    old_claim_id TEXT NOT NULL UNIQUE,
    new_claim_id TEXT
);

INSERT OR IGNORE INTO nexus_v8_claim_id_map(old_claim_id)
SELECT id FROM claims
WHERE length(CAST(id AS BLOB)) > 512
ORDER BY id;

INSERT OR IGNORE INTO nexus_v8_claim_id_map(old_claim_id)
SELECT left_claim_id FROM conflicts
WHERE length(CAST(left_claim_id AS BLOB)) > 512
ORDER BY left_claim_id;

INSERT OR IGNORE INTO nexus_v8_claim_id_map(old_claim_id)
SELECT right_claim_id FROM conflicts
WHERE length(CAST(right_claim_id AS BLOB)) > 512
ORDER BY right_claim_id;

UPDATE nexus_v8_claim_id_map
SET new_claim_id = (
    WITH RECURSIVE candidates(attempt, candidate) AS (
        SELECT 0, 'nexus-v8-claim-' || printf(
            '%016x', nexus_v8_claim_id_map.mapping_id
        )
        UNION ALL
        SELECT attempt + 1,
            'nexus-v8-claim-' || printf(
                '%016x', nexus_v8_claim_id_map.mapping_id
            ) || '-' || printf('%016x', attempt + 1)
        FROM candidates
        WHERE EXISTS (
            SELECT 1 FROM claims WHERE claims.id = candidates.candidate
            UNION ALL
            SELECT 1 FROM conflicts WHERE conflicts.left_claim_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM conflicts WHERE conflicts.right_claim_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM nexus_v8_claim_id_map AS assigned
            WHERE assigned.mapping_id <> nexus_v8_claim_id_map.mapping_id
              AND assigned.new_claim_id = candidates.candidate
        )
    )
    SELECT candidate FROM candidates ORDER BY attempt DESC LIMIT 1
);

UPDATE conflicts
SET left_claim_id = (
    SELECT new_claim_id FROM nexus_v8_claim_id_map
    WHERE old_claim_id = conflicts.left_claim_id
)
WHERE length(CAST(left_claim_id AS BLOB)) > 512;

UPDATE conflicts
SET right_claim_id = (
    SELECT new_claim_id FROM nexus_v8_claim_id_map
    WHERE old_claim_id = conflicts.right_claim_id
)
WHERE length(CAST(right_claim_id AS BLOB)) > 512;

UPDATE claims
SET id = (
    SELECT new_claim_id FROM nexus_v8_claim_id_map
    WHERE old_claim_id = claims.id
)
WHERE length(CAST(id AS BLOB)) > 512;

-- Tool-use IDs participate in the claims uniqueness key and are repeated in
-- advisory history. Give each legacy identity a shared bounded replacement.
CREATE TEMP TABLE nexus_v8_tool_id_map (
    mapping_id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    old_tool_use_id TEXT NOT NULL,
    new_tool_use_id TEXT,
    UNIQUE(project_id, session_id, old_tool_use_id)
);

INSERT OR IGNORE INTO nexus_v8_tool_id_map(project_id, session_id, old_tool_use_id)
SELECT project_id, session_id, tool_use_id FROM claims
WHERE length(CAST(tool_use_id AS BLOB)) > 512
ORDER BY project_id, session_id, tool_use_id;

UPDATE nexus_v8_tool_id_map
SET new_tool_use_id = (
    WITH RECURSIVE candidates(attempt, candidate) AS (
        SELECT 0, 'nexus-v8-tool-' || printf(
            '%016x', nexus_v8_tool_id_map.mapping_id
        )
        UNION ALL
        SELECT attempt + 1,
            'nexus-v8-tool-' || printf(
                '%016x', nexus_v8_tool_id_map.mapping_id
            ) || '-' || printf('%016x', attempt + 1)
        FROM candidates
        WHERE EXISTS (
            SELECT 1 FROM claims WHERE claims.tool_use_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM advisories WHERE advisories.tool_use_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM nexus_v8_tool_id_map AS assigned
            WHERE assigned.mapping_id <> nexus_v8_tool_id_map.mapping_id
              AND assigned.new_tool_use_id = candidates.candidate
        )
    )
    SELECT candidate FROM candidates ORDER BY attempt DESC LIMIT 1
);

UPDATE advisories
SET tool_use_id = (
    SELECT new_tool_use_id FROM nexus_v8_tool_id_map
    WHERE nexus_v8_tool_id_map.project_id = advisories.project_id
      AND nexus_v8_tool_id_map.session_id = advisories.session_id
      AND nexus_v8_tool_id_map.old_tool_use_id = advisories.tool_use_id
)
WHERE length(CAST(tool_use_id AS BLOB)) > 512
  AND EXISTS (
      SELECT 1 FROM nexus_v8_tool_id_map
      WHERE nexus_v8_tool_id_map.project_id = advisories.project_id
        AND nexus_v8_tool_id_map.session_id = advisories.session_id
        AND nexus_v8_tool_id_map.old_tool_use_id = advisories.tool_use_id
  );

UPDATE claims
SET tool_use_id = (
    SELECT new_tool_use_id FROM nexus_v8_tool_id_map
    WHERE nexus_v8_tool_id_map.project_id = claims.project_id
      AND nexus_v8_tool_id_map.session_id = claims.session_id
      AND nexus_v8_tool_id_map.old_tool_use_id = claims.tool_use_id
)
WHERE length(CAST(tool_use_id AS BLOB)) > 512;

-- A caller session identity can be repeated across projects and history
-- tables. Give each oversized text identity one collision-free replacement so
-- all surviving associations retain the same value.
CREATE TEMP TABLE nexus_v8_session_id_map (
    mapping_id INTEGER PRIMARY KEY AUTOINCREMENT,
    old_session_id TEXT NOT NULL UNIQUE,
    new_session_id TEXT,
    UNIQUE(new_session_id)
);

INSERT OR IGNORE INTO nexus_v8_session_id_map(old_session_id)
SELECT session_id FROM sessions
WHERE length(CAST(session_id AS BLOB)) > 512
ORDER BY session_id;

INSERT OR IGNORE INTO nexus_v8_session_id_map(old_session_id)
SELECT session_id FROM claims
WHERE length(CAST(session_id AS BLOB)) > 512
ORDER BY session_id;

UPDATE nexus_v8_session_id_map
SET new_session_id = (
    WITH RECURSIVE candidates(attempt, candidate) AS (
        SELECT 0, 'nexus-v8-session-' || printf(
            '%016x', nexus_v8_session_id_map.mapping_id
        )
        UNION ALL
        SELECT attempt + 1,
            'nexus-v8-session-' || printf(
                '%016x', nexus_v8_session_id_map.mapping_id
            ) || '-' || printf('%016x', attempt + 1)
        FROM candidates
        WHERE EXISTS (
            SELECT 1 FROM events WHERE events.session_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM sessions WHERE sessions.session_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM claims WHERE claims.session_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM advisories WHERE advisories.session_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM capability_uses
            WHERE capability_uses.session_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM memory_entries
            WHERE memory_entries.session_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM memory_activations
            WHERE memory_activations.session_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM papercuts
            WHERE papercuts.creator_session_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM papercut_upvotes
            WHERE papercut_upvotes.reporter_session_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM papercut_duplicate_reviews
            WHERE papercut_duplicate_reviews.reporter_session_id = candidates.candidate
               OR papercut_duplicate_reviews.reviewer_session_id = candidates.candidate
            UNION ALL
            SELECT 1 FROM nexus_v8_session_id_map AS assigned
            WHERE assigned.mapping_id <> nexus_v8_session_id_map.mapping_id
              AND assigned.new_session_id = candidates.candidate
        )
    )
    SELECT candidate FROM candidates ORDER BY attempt DESC LIMIT 1
);

UPDATE events
SET session_id = (
    SELECT new_session_id FROM nexus_v8_session_id_map
    WHERE old_session_id = events.session_id
)
WHERE EXISTS (
    SELECT 1 FROM nexus_v8_session_id_map
    WHERE old_session_id = events.session_id
);

UPDATE advisories
SET session_id = (
    SELECT new_session_id FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = advisories.session_id
)
WHERE EXISTS (
    SELECT 1 FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = advisories.session_id
);

UPDATE capability_uses
SET session_id = (
    SELECT new_session_id FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = capability_uses.session_id
)
WHERE EXISTS (
    SELECT 1 FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = capability_uses.session_id
);

DROP TRIGGER memory_entries_immutable_update;

UPDATE memory_entries
SET session_id = (
    SELECT new_session_id FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = memory_entries.session_id
)
WHERE EXISTS (
    SELECT 1 FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = memory_entries.session_id
);

CREATE TRIGGER memory_entries_immutable_update
BEFORE UPDATE ON memory_entries
BEGIN
    SELECT RAISE(ABORT, 'memory entries are immutable');
END;

UPDATE memory_activations
SET session_id = (
    SELECT new_session_id FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = memory_activations.session_id
)
WHERE EXISTS (
    SELECT 1 FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = memory_activations.session_id
);

UPDATE papercuts
SET creator_session_id = (
    SELECT new_session_id FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = papercuts.creator_session_id
)
WHERE EXISTS (
    SELECT 1 FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = papercuts.creator_session_id
);

UPDATE papercut_upvotes
SET reporter_session_id = (
    SELECT new_session_id FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = papercut_upvotes.reporter_session_id
)
WHERE EXISTS (
    SELECT 1 FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = papercut_upvotes.reporter_session_id
);

UPDATE papercut_duplicate_reviews
SET reporter_session_id = (
    SELECT new_session_id FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = papercut_duplicate_reviews.reporter_session_id
)
WHERE EXISTS (
    SELECT 1 FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = papercut_duplicate_reviews.reporter_session_id
);

UPDATE papercut_duplicate_reviews
SET reviewer_session_id = (
    SELECT new_session_id FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = papercut_duplicate_reviews.reviewer_session_id
)
WHERE EXISTS (
    SELECT 1 FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = papercut_duplicate_reviews.reviewer_session_id
);

UPDATE claims
SET session_id = (
    SELECT new_session_id FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = claims.session_id
)
WHERE length(CAST(session_id AS BLOB)) > 512;

UPDATE sessions
SET session_id = (
    SELECT new_session_id FROM nexus_v8_session_id_map
    WHERE nexus_v8_session_id_map.old_session_id = sessions.session_id
)
WHERE length(CAST(session_id AS BLOB)) > 512;

-- Keep the longest valid UTF-8 prefix for human-readable legacy fields.
UPDATE sessions
SET agent = (
    WITH RECURSIVE bounds(low, high) AS (
        VALUES(0, MIN(length(sessions.agent), 256))
        UNION ALL
        SELECT
            CASE WHEN length(CAST(substr(sessions.agent, 1, (low + high + 1) / 2) AS BLOB)) <= 256
                 THEN (low + high + 1) / 2 ELSE low END,
            CASE WHEN length(CAST(substr(sessions.agent, 1, (low + high + 1) / 2) AS BLOB)) <= 256
                 THEN high ELSE ((low + high + 1) / 2) - 1 END
        FROM bounds WHERE low < high
    )
    SELECT substr(sessions.agent, 1, MAX(low)) FROM bounds
)
WHERE length(CAST(agent AS BLOB)) > 256;

UPDATE sessions
SET worktree = (
    WITH RECURSIVE bounds(low, high) AS (
        VALUES(0, MIN(length(sessions.worktree), 4096))
        UNION ALL
        SELECT
            CASE WHEN length(CAST(substr(sessions.worktree, 1, (low + high + 1) / 2) AS BLOB)) <= 4096
                 THEN (low + high + 1) / 2 ELSE low END,
            CASE WHEN length(CAST(substr(sessions.worktree, 1, (low + high + 1) / 2) AS BLOB)) <= 4096
                 THEN high ELSE ((low + high + 1) / 2) - 1 END
        FROM bounds WHERE low < high
    )
    SELECT substr(sessions.worktree, 1, MAX(low)) FROM bounds
)
WHERE worktree IS NOT NULL
  AND length(CAST(worktree AS BLOB)) > 4096;

UPDATE sessions
SET task_summary = (
    WITH RECURSIVE bounds(low, high) AS (
        VALUES(0, MIN(length(sessions.task_summary), 1024))
        UNION ALL
        SELECT
            CASE WHEN length(CAST(substr(sessions.task_summary, 1, (low + high + 1) / 2) AS BLOB)) <= 1024
                 THEN (low + high + 1) / 2 ELSE low END,
            CASE WHEN length(CAST(substr(sessions.task_summary, 1, (low + high + 1) / 2) AS BLOB)) <= 1024
                 THEN high ELSE ((low + high + 1) / 2) - 1 END
        FROM bounds WHERE low < high
    )
    SELECT substr(sessions.task_summary, 1, MAX(low)) FROM bounds
)
WHERE task_summary IS NOT NULL
  AND length(CAST(task_summary AS BLOB)) > 1024;

UPDATE conflicts
SET message = (
    WITH RECURSIVE bounds(low, high) AS (
        VALUES(0, MIN(length(conflicts.message), 8192))
        UNION ALL
        SELECT
            CASE WHEN length(CAST(substr(conflicts.message, 1, (low + high + 1) / 2) AS BLOB)) <= 8192
                 THEN (low + high + 1) / 2 ELSE low END,
            CASE WHEN length(CAST(substr(conflicts.message, 1, (low + high + 1) / 2) AS BLOB)) <= 8192
                 THEN high ELSE ((low + high + 1) / 2) - 1 END
        FROM bounds WHERE low < high
    )
    SELECT substr(conflicts.message, 1, MAX(low)) FROM bounds
)
WHERE length(CAST(message AS BLOB)) > 8192;

DROP TABLE nexus_v8_path_map;
DROP TABLE nexus_v8_claim_id_map;
DROP TABLE nexus_v8_tool_id_map;
DROP TABLE nexus_v8_session_id_map;

CREATE TRIGGER sessions_projection_width_insert
BEFORE INSERT ON sessions
WHEN length(CAST(NEW.session_id AS BLOB)) > 512
  OR length(CAST(NEW.agent AS BLOB)) > 256
  OR (NEW.worktree IS NOT NULL AND length(CAST(NEW.worktree AS BLOB)) > 4096)
  OR (NEW.task_summary IS NOT NULL AND length(CAST(NEW.task_summary AS BLOB)) > 1024)
BEGIN
    SELECT RAISE(ABORT, 'session projection exceeds storage width');
END;

CREATE TRIGGER sessions_projection_width_update
BEFORE UPDATE OF session_id, agent, worktree, task_summary ON sessions
WHEN length(CAST(NEW.session_id AS BLOB)) > 512
  OR length(CAST(NEW.agent AS BLOB)) > 256
  OR (NEW.worktree IS NOT NULL AND length(CAST(NEW.worktree AS BLOB)) > 4096)
  OR (NEW.task_summary IS NOT NULL AND length(CAST(NEW.task_summary AS BLOB)) > 1024)
BEGIN
    SELECT RAISE(ABORT, 'session projection exceeds storage width');
END;

CREATE TRIGGER claims_projection_width_insert
BEFORE INSERT ON claims
WHEN length(CAST(NEW.id AS BLOB)) > 512
  OR length(CAST(NEW.session_id AS BLOB)) > 512
  OR length(CAST(NEW.tool_use_id AS BLOB)) > 512
  OR length(CAST(NEW.path AS BLOB)) > 4096
BEGIN
    SELECT RAISE(ABORT, 'claim projection exceeds storage width');
END;

CREATE TRIGGER claims_projection_width_update
BEFORE UPDATE OF id, session_id, tool_use_id, path ON claims
WHEN length(CAST(NEW.id AS BLOB)) > 512
  OR length(CAST(NEW.session_id AS BLOB)) > 512
  OR length(CAST(NEW.tool_use_id AS BLOB)) > 512
  OR length(CAST(NEW.path AS BLOB)) > 4096
BEGIN
    SELECT RAISE(ABORT, 'claim projection exceeds storage width');
END;

CREATE TRIGGER conflicts_projection_width_insert
BEFORE INSERT ON conflicts
WHEN length(CAST(NEW.left_claim_id AS BLOB)) > 512
  OR length(CAST(NEW.right_claim_id AS BLOB)) > 512
  OR length(CAST(NEW.path AS BLOB)) > 4096
  OR length(CAST(NEW.message AS BLOB)) > 8192
BEGIN
    SELECT RAISE(ABORT, 'conflict projection exceeds storage width');
END;

CREATE TRIGGER conflicts_projection_width_update
BEFORE UPDATE OF left_claim_id, right_claim_id, path, message ON conflicts
WHEN length(CAST(NEW.left_claim_id AS BLOB)) > 512
  OR length(CAST(NEW.right_claim_id AS BLOB)) > 512
  OR length(CAST(NEW.path AS BLOB)) > 4096
  OR length(CAST(NEW.message AS BLOB)) > 8192
BEGIN
    SELECT RAISE(ABORT, 'conflict projection exceeds storage width');
END;
