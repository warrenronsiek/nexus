---
feature: agent-memory
---

# Agent memory

## What

Agent memory is an explicit, durable note store with two streams: one global stream and one project stream. Local mode derives the project identity from the Git common directory; remote mode uses the client's explicit project ID so different clones can share a stream. Raw notes are authoritative, immutable, ordered, and attributed to their authoring agent, session, model, and configuration. Derived summaries form a disposable power-of-two hierarchy over each stream.

The caller-facing contract is deliberately small. Agents may add a single-line note with an explicit scope, search raw notes, or expand a summary. Nexus owns consolidation, retries, fallback providers, activation, and summary invalidation.

## Why

Long-running agent work benefits from selected historical facts without replaying complete transcripts or making the caller maintain a summary tree. Explicit capture avoids silently treating arbitrary prompts, model output, or tool output as durable truth. Immutable source notes keep derivation auditable, while bounded hierarchical context keeps old history useful without crowding out recent detail.

## Data flow

1. An agent explicitly adds a normalized global or project note of at most 512 UTF-8 bytes. The MCP adapter binds each write to its established process or HTTP-session identity and working directory, replacing per-call identity fields before decoding. Host lifecycle observation independently records the deliberate tool call. No shared last-caller state is used.
2. The selected database transactionally assigns its stream ordinal and collapses an exact normalized duplicate in the same stream. SQLite additionally rolls back a reader-blocked commit before retrying the complete write within its two-second lock retry budget. Failed commits cannot leave the shared connection inside an unfinished transaction.
3. Every 15 seconds, Nexus selects at most eight ready missing summary nodes from one tree level and snapshots their source hashes. Periodic and user-requested consolidation share one service lease, so overlapping requests do not launch duplicate provider batches.
4. Nexus releases the database lock and requests strict JSON summaries from the configured analyst provider. The request exposes the same 2,048 UTF-8 byte maximum enforced by validation and targets 1,024 bytes and 100 words or fewer, prioritizing decisions that affect future work rather than enumerating every source detail. Failure, timeout, or invalid output causes one attempt with the other configured provider.
5. Nexus transactionally rechecks source hashes before accepting summaries. Attempts record provider and sanitized outcome metadata, never memory text.
6. The first user prompt in an agent session and memory scope receives bounded, clearly delimited global and project context as untrusted historical data. Activation identity includes the read scope and project namespace, so the same host-assigned session ID used in another project still receives that project's context. Explicit context reads do not consume activation.

Failed attempts cool down independently for each provider and source range. The first retry delay is 15 seconds, each consecutive failure doubles it, and the delay is capped at one hour. A successful attempt resets that provider's consecutive-failure count. If both providers fail for the same still-pending source range, health is degraded until a later attempt succeeds or the source range changes.

Lifecycle delivery remains bounded and fail open. Consolidation never runs in a lifecycle-hook request path.

Frontier snapshots and the ready-node queue are disposable derived accelerators. Writes advance them transactionally when possible; a bounded background repair republishes stale snapshots after interruption, including on ticks with no consolidation jobs. A lifecycle read never rebuilds history or waits for a provider: it skips stale context without consuming the session activation so a later prompt can retry after repair.

## Reading memory

Layered context allocates one third of its budget to global memory and two thirds to project memory, donating unused capacity to the other scope. Each stream starts as ordered raw leaves and repeatedly replaces the oldest available aligned sibling pair with its stored parent summary. The resulting frontier is capped at 64 items and 16 KiB.

If a required summary is missing, Nexus omits the oldest excess range, reports the omission, and preserves recent memory. Raw-note search accepts at most 4 KiB of regex source, then performs a bounded, deterministic, newest-first case-insensitive Rust regex search over the newest 4,096 raw entries in each selected scope, returning at most 64 matches. The database query applies that candidate ceiling before materializing rows, and stored provenance rejects control characters while bounding agent/model/config identities to 256 bytes and session identity to 512 bytes. Expanding a summary returns its two children, which may themselves be raw notes or summaries.

Raw notes cannot be edited, deleted, corrected, retracted, or purged. Invalidating a summary deletes only that derived node and its stored ancestors; every source note remains intact. The V8 persistence upgrade may rewrite only an oversized legacy session attribution to the collision-free bounded identity shared by the corresponding session projection; it temporarily removes and restores the database immutability trigger inside the migration transaction while leaving note content and order unchanged.

## Interfaces

The CLI exposes status, context, add, search, expand, invalidate, and immediate consolidation operations. MCP exposes only `nexus_memory_add`, `nexus_memory_search`, and `nexus_memory_expand` to caller agents. Codex and Claude receive first-prompt context through bounded lifecycle `additionalContext`; Pi receives it through `before_agent_start` using its correlated Nexus companion. The companion follows local Unix-socket or remote authenticated HTTP configuration without changing the tool surface. Consolidation providers use the shared model adapter, whose nonblocking byte-bounded stdout and stderr capture is implemented once in the crate-private `process_capture` module.

The Memory tab in `nexus tui` and Pi's `/nexus` shows raw and summary nodes, pending work, provider health, fallback use, activation time, and scope counts. Both hosts share raw search, scoped note creation, summary expansion, confirmed summary invalidation, an immediate consolidation retry, and refresh through the same typed memory API and prompt policy. Their companions start with explicit `nexus-tui` or `pi` agent identities, so note provenance comes from the bound MCP process rather than hidden tool arguments. Raw note operations require no model credentials. In a remote deployment, consolidation runs inside `nexus-server`, where provider execution is disabled by default to protect the service credential from same-UID subprocesses. It is available only with the explicit `--allow-local-providers` escape hatch and an externally isolated provider command. Missing or denied providers do not prevent adding, searching, expanding, or activating already available raw memory.

## Attribution

Nexus agent memory is inspired by Victor Taelin’s [OptMem](https://github.com/VictorTaelin/OptMem). Nexus uses an independent Rust/Diesel implementation with SQLite and PostgreSQL backends and includes no OptMem source.

The design review used OptMem commit [`1fb164c`](https://github.com/VictorTaelin/OptMem/tree/1fb164cf39028047781f72ac3bb1e5a691c1dcb0). That revision has no declared license and the upstream [license question remains open](https://github.com/VictorTaelin/OptMem/issues/8), so Nexus does not vendor, translate, or preserve its code or file format.
