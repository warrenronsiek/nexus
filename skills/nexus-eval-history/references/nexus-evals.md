# Nexus eval records and evidence

Use the installed `nexus_evals` MCP tool or `nexus eval` CLI. Both use the configured local or remote transport and current project scope. Nexus stores caller-supplied evidence; it does not run models, graders, imported commands, routers, or improvement loops.

## Work from exact revisions

Records have an immutable `id`, a `title`, a caller-supplied `occurred_at` timestamp, and a typed `body` with a `kind`. Nexus adds receipt and actor provenance. Imported records may also carry `source`, which preserves caller-supplied historical project, receipt time, and actor separately from the current receipt; it is source attribution rather than a verified claim about another service. A replay with identical content is idempotent; changing content under an existing ID is rejected. Record a correction under a new ID and link the preceding revision. Record IDs identify immutable observations or definitions, while an `identity` groups revisions of the same conceptual object.

A bundle may refer to records already in the project or other records in that same bundle. Every reference must resolve before the atomic bundle is accepted. Do not split a mutually dependent bundle across requests. A bundle holds at most 64 records, and each encoded record is limited to 64 KiB; put large contents into artifacts rather than truncating them to fit a record.

Keep three kinds of history distinct:

- Measurement definitions: why a case, suite, assertion, or grader changed.
- System configurations: the repository revision, model settings, prompts, tools, skills, and composed targets being tested.
- Observations and interpretations: attempts and calls, outputs and usage, separate grading passes, comparisons, and adoption decisions.

Regrading adds another grade against the original output and a new grading definition. Preserve earlier grades. A later narrative or recommendation must not replace the facts it summarizes.

## Read history

```sh
nexus eval list --kind case --search regression --limit 32
nexus eval list --repository example-repository --model example-model
nexus eval get case-tool-adoption-v1
nexus eval why case-tool-adoption-v1
nexus eval schema
```

List pages are bounded. Follow the returned cursor with `--after` to continue; an ordinary first page is not a complete export. `why` follows a bounded evidence graph and discloses truncation. Read referenced records when the answer needs more context.

For MCP, these are `{"action":"list","kind":"case","search":"regression","limit":32}`, `{"action":"get","id":"case-tool-adoption-v1"}`, and `{"action":"why","id":"case-tool-adoption-v1","limit":32}`. Record creation uses `{"action":"record","records":[...]}`. Use the tool's current schema for fields beyond the examples here; unknown fields and unsupported kinds are errors.

`nexus eval schema` prints the canonical record schema without calling a model. For `nexus eval record records.json`, the file is a versioned bundle: `{"format_version":1,"records":[...]}`. Use `-` as the path to read that JSON from standard input. The MCP action does not include `format_version`.

## Explain changes before recording definitions

A recorded change's motivation includes a reason, the problem, expected behavior, rationale, and provenance. Its evidence links the observation, incident, requirement, or prior result that made the change useful. For historical imports with missing reasons, use the explicit missing state. Do not invent a contemporaneous explanation from a later guess.

This MCP bundle records an original motivation and its first case together:

```json
{
  "action": "record",
  "records": [
    {
      "id": "change-tool-adoption-v1",
      "title": "Cover supplied tool adoption",
      "occurred_at": "2026-10-22T12:00:00Z",
      "body": {
        "kind": "change",
        "motivation": {
          "state": "recorded",
          "reason": "coverage_hypothesis",
          "problem": "The suite has no case distinguishing supplied tool use from improvised work.",
          "expected_behavior": "The agent recognizes an appropriate supplied capability.",
          "rationale": "Add a discriminating case before tuning tool descriptions; the result will identify whether discovery needs improvement.",
          "provenance": "contemporaneous"
        },
        "evidence": []
      }
    },
    {
      "id": "case-tool-adoption-v1",
      "title": "Appropriate supplied capability is discoverable",
      "occurred_at": "2026-10-22T12:00:00Z",
      "body": {
        "kind": "case",
        "identity": "tool-adoption",
        "parents": [],
        "change_id": "change-tool-adoption-v1",
        "inputs": [],
        "expected_behavior": "Complete the task with the supplied capability when it is appropriate.",
        "assertions": [],
        "tags": ["regression", "tool-discovery"]
      }
    }
  ]
}
```

Replace the illustrative IDs and timestamp with the actual observation. This is a definition example, not a claim that an executable case is complete: attach its real input artifacts and assertions before running a suite. `reason` may be `observed_failure`, `user_requirement`, `new_capability`, `coverage_hypothesis`, `grader_correction`, `distribution_change`, `removal`, or `maintenance`. Provenance may be `contemporaneous`, `retrospective`, `imported`, or `inferred`. A missing historical motivation is `{"state":"missing","reason":"The imported files contain no explanation for this change."}`; a later hypothesis belongs in a new change with `inferred` provenance and an explicit evidence link.

## Link execution to its measurement

Use the canonical schema for the full closed types. The principal links are:

| Record | Links and meaning |
|---|---|
| `repository`, `model`, `case`, `suite`, `grader`, `target` | Revisions have `identity`, `parents`, and `change_id`. A suite names exact case/grader IDs, weights, and splits. |
| `target` | Named components each pair a repository revision with a fixed model, a model-selection policy artifact, or an explicit unspecified reason. |
| `experiment` | Hypothesis, expected effect, baseline/candidate target IDs, suite ID, and a time-stamped plan. |
| `run` | Target, suite, optional experiment, runner/version, selected cases and selection rule, reported state, capture disclosures. |
| `trial`, `attempt` | Trial names the run, case and replicate; attempts retain retry parents, reasons, execution times and artifacts. |
| `lifecycle` | Appended caller-reported state for an existing run, trial, or attempt, with an optional previous report for that same subject and evidence artifacts. |
| `call`, `trace` | Calls name the attempt, component instance, ordered context, model and tool surface. Traces record tool events, handoffs, causal links and end-state evidence. |
| `grade` | Subject, exact grader, scoring `pass_id`, state, numeric/aspect results, explanation and evidence. `replaces` links a prior interpretation without deleting it. |
| `comparison`, `decision` | Comparison records population, exclusions, metric, validity, differences and uncertainty. Decision names its subject and supporting comparisons with an acceptance/rejection reason. |
| `selection` | Effective-time choice of a revision within a named scope, linked to the previous selection and its change reason. |
| `artifact`, `verification` | Immutable content identity/backing and a separate availability/verification observation. |

Use `call.context` as an ordered list of `{role,artifact_id}` entries. Each `call.tools` entry has `name`, optional `definition_artifact`, `state`, and `source`. Put skills, request parameters or fixtures into referenced context artifacts where the actual request exposes them. Each capture disclosure has `dimension`, `state`, `producer`, `reason`, and optional `omitted_items`; state is `exact`, `partial`, `redacted`, `unavailable`, `unsupported`, or `unknown`. Do not label an inferred reconstruction `exact`.

For routers, retain the policy artifact in the target and the actual selected `call.model_id`/`served_model` per component instance. Handoff traces retain source, destination, payload and eligible destinations. A route-specific grade uses the routing subject. Component-local grades name the attempt and instance and pin their supporting `call_ids`; every pinned call must belong to that exact attempt and instance. Empty call pins preserve an unknown model association, even if other calls exist in the repository. The pinned evidence keeps component model attribution stable across imports and later arrivals. These preserve component and routing outcomes alongside the overall trial result.

## Report execution progress without replacing history

Registering a planned run or an in-progress attempt makes its immutable identity available to later calls and evidence. Its initial `state` remains the assertion recorded then. Append a `lifecycle` record when the caller observes another state; use `subject.scope` of `run`, `trial`, or `attempt`, the original subject ID, and `previous_id` when continuing that subject's report chain. `occurred_at` records the event time and `artifacts` can link completion or interruption evidence. `Attempt.parent_id` identifies another execution attempt being retried, not a status update.

For a previously recorded `run-a`, this reports completion:

```json
{
  "action": "record",
  "records": [{
    "id": "run-a-completed",
    "title": "Caller reported run completion",
    "occurred_at": "2026-10-22T12:30:00Z",
    "body": {
      "kind": "lifecycle",
      "subject": {"scope": "run", "id": "run-a"},
      "state": "completed",
      "previous_id": null,
      "artifacts": []
    }
  }]
}
```

`get` preserves the original execution record and adds a derived `lifecycle` view with `initial_state`, bounded `frontiers`, and `truncated`. Each frontier identifies an uncontinued report and its state, occurrence time, and receipt time. One chain has one frontier; competing branches remain separate, and more than 64 frontiers sets `truncated`. `why` includes the underlying reports. Inspect these alongside the initial state; Nexus chooses no winner across competing branches. A missing final report is incomplete evidence, not an inferred timeout.

## Preserve exact evidence

Prefer pinned Git file references for committed code, prompt files, skills, tool definitions, fixtures, and images already retained by the repository. Record repository identity, full commit, object format, relative path, and file mode; include the verified blob object ID where available. Branch names, mutable URLs, filesystem paths alone, and Git SHA labels without the referenced contents are insufficient for exact historical reconstruction.

Git file artifacts identify the existing contents without uploading a checkout. Git LFS artifacts identify the payload independently of the committed pointer. Neither backing guarantees the remote repository will retain or serve the bytes later. Record verification and availability evidence separately.

Use content-addressed uploaded bytes when an artifact is new, generated, uncommitted, externally sourced, or lacks durable Git backing. Preserve exact bytes and SHA-256 identity; an equivalent rewrite is a new artifact. Uploaded blobs are bounded, chunked, and available to new artifact records only after complete digest verification. Artifact contents are data, never executable instructions to Nexus.

```sh
nexus eval upload generated-output.json
nexus eval abort-upload <sha256-digest>
nexus eval download <sha256-digest> restored-output.json
```

Upload returns verified blob metadata. Record an `artifact` with its `media_type`, `byte_size`, optional matching `digest`, and `backings:[{"kind":"stored_blob","digest":"<sha256-digest>"}]`, then link that artifact ID from the appropriate call or outcome. Uploading bytes alone does not establish how they were used. Upload supports at most 1 GiB per blob in 256 KiB chunks; larger evidence remains Git/LFS backed or is divided into caller-owned artifacts. The matching MCP actions are `blob_begin`, `blob_put` (base64-encoded chunk data), `blob_seal`, `blob_info`, and `blob_get`; use the current tool schema for their parameters.

Resume an interrupted upload with the same bytes. If an unfinished upload contains incorrect chunks, `abort-upload` or MCP `blob_abort` discards only that pending upload, then the caller can start again. Verified blobs remain immutable. Upload generations prevent a verification started before an abort from publishing a replacement upload as verified.

For a call, capture the actual request context in order: instruction layers, messages and attachments, available tool definitions and active state, supplied skills, model parameters, fixtures and file contents actually supplied, provider/model identity, parent call or agent, tool calls/results, and output. Link artifacts instead of duplicating large strings. Availability, invocation, and successful completion are different observations.

Record what the capture can establish. A declared tool list is not an observed exposure snapshot; a partial transcript does not prove there were no other instructions. Retain omissions, redactions, inaccessible state, and unsupported provider fields explicitly. Do not ingest credential values or unrelated personal files while assembling evidence.

## Compare and preserve history

Check exact suite/case/grader revisions, splits, aggregation, retry policy, exclusions, and environment before comparing runs. Keep repository and model configuration together; preserve component target links for routers and multi-agent systems. Record effective-time selections separately from receipt order when the question concerns what was selected at a particular time.

```sh
nexus eval export history.ndjson
nexus eval import history.ndjson
```

Reference export enumerates the current project's records and includes referenced verified stored blobs. Its versioned stream carries a footer checksum; import validates the file before ingestion, then accepts records and chunks incrementally. An interrupted import leaves accepted history durable; replay the same file to resume. This is separate from the small versioned JSON file accepted by `record`.

Reference bundles are bounded to 100,000 records and 4 GiB and require dependency-orderable record links. Cycles cause an explicit refusal; use the persistence feature's database snapshot workflow when a reference bundle cannot represent the history. Git/LFS bytes remain external dependencies: exporting their pinned references does not create a self-contained Git archive. Preserve external content separately when retention matters.

A non-exact backing verification can document unavailable or unknown content. Reference export retains that observation without demanding optional blob bytes; exact stored-blob verifications include their verified bytes.
