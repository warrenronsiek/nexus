---
feature: evals
---

# Evaluation history

## What

Nexus retains the caller's evaluation definitions, motivations, reported executions,
assessment passes, comparisons, and decisions as immutable project-scoped records.
It stores what was tried and why it changed. Callers own execution, grading,
sampling, retries, budgets, routing, and improvement loops. Nexus never executes
an imported artifact or launches an evaluation.

## Why

A score alone cannot explain what it measured or why a case existed. Typed links
connect immutable case, suite, grader, metric, repository, model, and target revisions to
the changes that motivated them, then to runs and individual calls, grades, and
decisions. Historical missing rationale remains an explicit gap.

## Data flow

```mermaid
flowchart TD
    C[Caller records definitions and evidence] --> V[Validate typed records and references]
    V --> T[Atomic immutable bundle publication]
    T --> D[(SQLite or PostgreSQL)]
    G[Pinned Git file or LFS identity] --> V
    B[Necessary new bytes] --> U[Bounded resumable chunk upload]
    U --> H[Verify complete SHA-256 content]
    H --> D
    D --> Q[Bounded list, detail, and why traversal]
    D --> E[Portable export with external Git dependencies]
```

The caller supplies closed typed records and its own occurrence times. Nexus
binds project and actor provenance, validates identifiers and content bounds, and
checks every internal link before publishing the complete bundle. Existing
identical records replay without changing their original attribution; conflicting
immutable content fails the entire transaction. The selected database is the
authority for recorded facts and uploaded bytes. Query projections expose their
limits and never interpret artifact bytes as instructions.

## Implementation

`src/evals/` owns public record types, their closed variants, relationship
derivation, and validation. `Store::record_eval_bundle` publishes records and
indexed links/dimensions through Diesel's typed DSL. `Store::list_eval_records`
uses opaque ID cursors with a server receipt-time cutoff and bounded
project/kind/repository/model/text filters. Later ingestion cannot enter an
already-started traversal, including records with earlier occurrence times.
`Store::get_eval_record` returns the original typed body and provenance.
`Store::why_eval_record` follows the recorded evidence graph with a bounded
visited set, including incoming superseding explanations/decisions, regrading,
artifact verifications, and caller-reported lifecycle events. Ordinary incoming
run references are not expanded.
A truncated graph is explicitly marked.

Metric revisions retain meaning, arbitrary unit and value-format descriptions,
optional value-schema and interpretation text, and a calculation definition with
description, optional formula, pinned source artifacts, population, aggregation,
exclusions, and missing-data policy. Every revision links its motivating change.
Measurements name an exact metric revision and assessment subject (including a
whole run), explanation, input records, and artifact evidence. Their value is
opaque exact inline text, an artifact reference, or an unavailable reason. Inline
text preserves empty values, large integers, decimal precision, JSON, CSV, and
caller-defined representations without numerical coercion. Nexus neither parses
the declared format nor validates it against the optional schema, executes its
calculation, normalizes units, or enforces an optimization policy.

Comparisons may name exact metric revisions, measurement records, or both;
measurement links already resolve their definitions. Historical
inline numeric grades and comparison metrics remain supported with their original
wire representation; that float path is optional for new comparisons. Definition
changes append motivated metric revisions. Correcting a measurement appends a
replacement with the same metric revision and subject, preserving the original
value. `why` follows definitions, calculation inputs/sources, and incoming
measurement corrections. Units or names alone establish no semantic equivalence.

Git is the default backing for committed code, prompts, tool definitions, skills,
fixtures, and images. Pinned references include repository, full commit, object
format, relative path, and optional verified blob identity. LFS payload identities
remain distinct from pointer identities. Repository/model/target links preserve
pair histories and arbitrary named target components. A Git reference does not
upload a checkout, fetch a URL, execute filters, or guarantee external retention.
Capture provenance and later verification are independent records.

New bytes use SHA-256 content identities and at most 256 KiB per chunk. Begin is
idempotent by project/digest/length; chunks are immutable and conflicting retries
fail. Only a complete digest-verified upload can back a newly accepted artifact.
Chunk retrieval exposes verified bytes only. Empty content is supported. The
caller may explicitly abort an unverified upload and retry corrupt or interrupted
content. Verified evidence cannot be aborted. Each upload has a server-generated
UUID retained in portable transfer; verification must publish against that same
generation, so aborting and beginning another upload cannot reuse an older
verification result.

The per-blob limit is 1 GiB. Service verification reads each immutable chunk under
a short store guard, hashes outside the guard, and publishes only after checking
the declared count. Larger content remains Git/LFS backed or is divided into
caller-owned artifacts. This limit is explicit rather than silent truncation.

Records accept at most 64 inputs per bundle, 64 KiB per encoded record, 128 direct
links, and 64 items per ordinary page or why traversal. Histories can be retrieved
over successive pages. A record's score, attempt status, declared capture state,
and artifact availability remain separate. Regrading adds a new grade record
and retains the old grader and outputs. Selection events identify effective and
receipt time without selecting an unknown fork automatically.

Component-local grades pin their supporting call IDs. Every pinned call must name
the grade's exact attempt and runtime instance. Model filters follow only those
calls; empty pins leave the model association unknown. Target declarations,
sibling calls, later ingestion, and import order cannot assign a model to that
component's score. Repository participation remains linked through the attempt.

Revision ancestry must be acyclic and retain its stable identity. A trial's case
must belong to the run's pinned suite. Retry and call parents retain their trial
and attempt scope respectively; regrades retain their assessment subject, and a
selection predecessor must name the same selection scope. These constraints are
checked before publication and again during portable import. All temporal
predecessor chains must also be acyclic; ancestry validation is bounded to 4096
records for both temporal chains and revision graphs.

Run, trial, and attempt state changes use immutable `lifecycle` records linking
the subject and optional previous event for that exact subject. Original reported
states remain unchanged. Event occurrence and receipt times remain distinct;
Nexus records the caller's report without inferring a timeout or executing work.
Ordinary detail reads include a derived `lifecycle` view with the initial state
and up to 64 report frontiers: reports not continued by another report. A linear
history exposes its latest report, while competing branches remain separate
frontiers with their record IDs, states, occurrence and receipt times. More than
64 frontiers sets `truncated`; Nexus chooses no winner and rewrites no facts.

Portable format 4 includes records, their links and filter dimensions, upload
metadata, and chunks. Format 2 and 3 imports remain supported. Git dependencies
remain external in these reference-preserving database snapshots; export does
not claim a self-contained Git archive. Stream tables use extensible visited
tracking and encoded-byte-aware batches. Existing snapshot size limits remain
explicit. Equivalent V12 migrations support SQLite and PostgreSQL.
