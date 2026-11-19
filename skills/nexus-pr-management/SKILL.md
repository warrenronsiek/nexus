---
name: nexus-pr-management
description: Use when asked to shepherd a PR, promote a release, or coordinate delivery. Register and release responsibility, trace original contributors, assess promotion dependencies, and route addressed remediation with a verified receipt.
---

# Keep implementation context through delivery

Use Nexus around the repository's existing PR, CI, and deployment workflow.
The shepherd coordinates the rollout. Original contributors retain the context
behind their changes. Nexus connects both to exact revisions and records what was
observed; the caller owns Git execution, release policy, and host communication.

Discover `nexus_delivery` and its input schema, or run `nexus delivery schema`
and the relevant subcommand's `--help`. CLI and MCP use the same typed observations
and reads. These are deliberate records: check the acknowledgment rather than
assuming an advisory hook proves successful delivery.

## Own the complete shepherd lifecycle

When the user asks you to shepherd or promote a release, registration and cleanup
are part of that task. Session capture does not register delivery responsibility.
Before the first promotion or deployment, discover existing responsibility in the
actual repository/project and register your assignment using your real host
session identity. For work spanning several PRs or environments, register and own
one rollout; a rollout can exist before its promotion PRs do. Retain its ID in the
caller's existing release ledger and link the PRs and deployment attempts as they
become known. Missing implementation provenance stays qualified as unknown.

Release the assignments you own when the requested work has finished or the user
explicitly cancels it. A PR merge alone is insufficient when your responsibility
includes post-merge deployment, seed, replay, or health validation. Pausing,
losing a tool connection, ending a host session, or encountering a retryable
failure is not completion; retain the assignment or explicitly hand it off.

At closure, reread each owned PR/rollout and submit `shepherd_assigned` with
`agent: null`, its current `assignment_event_id` as `expected_assignment`, and a
reason tied to the confirmed completion or cancellation. Only release a record
whose current shepherd is your agent/session. If another agent has taken over,
leave that assignment intact. Use the normal immutable observation envelope:

```json
{
  "event": "shepherd_assigned",
  "subject": {"kind": "rollout", "id": "ROLLOUT_ID"},
  "expected_assignment": "CURRENT_ASSIGNMENT_EVENT_ID",
  "agent": null,
  "reason": "Requested deployment and acceptance checks completed; release active responsibility."
}
```

Read the record back and confirm `shepherd` is null before claiming cleanup.
This removes active responsibility, not the PR, rollout, original contributors,
or assignment history. Stop only watches owned exclusively by the completed task;
shared watches retain their own lifecycle. If Nexus is unavailable, continue the
authorized release workflow and report the unconfirmed registration or cleanup
separately; never repeat deployment work to obtain a recording acknowledgment.

## Register implementation work

Keep ordinary edit/overlap capture in the installed host hooks. Claims expire
and confer no authorship or exclusive ownership; use
[coordination](../nexus-coordination/SKILL.md) for current overlap.

At a meaningful contribution boundary, record the original agent and session,
bounded rationale and constraints, exact base/head Git revisions, branch/worktree,
feature tags, and known logical work/activity identifiers. The convenience command
captures Git scope from the actual checkout:

```sh
nexus delivery contribute --id cache-isolation --title 'Isolate request caches' \
  --rationale 'Prevent requests from sharing cached credentials' \
  --repository acme/service --base BASE_OBJECT_ID --head HEAD_OBJECT_ID
```

Use the actual full object IDs and repository identity, not floating branch names.
Read the command's help for feature, constraint, and work-context flags. Caller
identity comes from the bound host context.
For shell integration, propagate the actual `NEXUS_AGENT` and
`NEXUS_HOST_SESSION_ID`, or the exact `NEXUS_WORK_CONTEXT` handoff described in
the [work integration guide](../nexus-work-history/references/integration.md).
A standalone CLI invocation without host context defaults to `nexus-cli` and a
generated command session, which does not identify the original implementation
chat. Bound MCP supplies reporter context; a recorded contribution's `origin`
must still name the actual known agent/session. Do not invent missing identity.
When recording directly, use `contribution_registered` and `revision_mapped`
observations. A mapping names the source contribution, resulting revision,
source revisions, Git operation, verification, and changed hunks. The mapping ID
is local to that contribution; another contribution may reuse the same label.
Retain both the contribution ID and mapping ID when referring to its evidence. Preserve
supplied, mechanically verified, model suggested, and unknown evidence distinctly.

Squashing, rebasing, cherry-picking, or resolving a merge needs an explicit
result-to-source mapping. A commit author, branch label, shared file, or shared
feature alone does not establish which agent designed a change. Retain multiple
contributors when evidence overlaps. Do not invent historical mappings.

## Find or become the shepherd

Before coordinating a PR, discover existing registrations:

```sh
nexus delivery list --kind pull_request --repository acme/service
nexus delivery list --kind pull_request --repository acme/service --branch staging
nexus delivery get pull_request PR_RECORD_ID
```

List can filter by state, feature, or target where recorded. Follow returned
cursors; an empty page is not proof that no PR exists in another scope.

Register the PR using `pull_request_registered`, including provider/number/URL,
full base/head revisions, source contribution IDs, and a rollout ID if known.
Provider/repository/number identifies one PR within the project. A duplicate
registration reports the existing PR ID; reuse it instead of creating another
shepherd registry entry.
Then record `shepherd_assigned` against that PR or rollout subject. Supply the
current `assignment_event_id` as `expected_assignment`; use null for the first
assignment. This prevents a stale agent from silently overwriting a newer handoff.
Use the actual caller agent/session identity and a reason for responsibility.

The convenience command registers the caller or an explicit successor:

```sh
nexus delivery shepherd pull_request PR_RECORD_ID --reason 'Coordinate staging promotion'
nexus delivery shepherd pull_request PR_RECORD_ID \
  --expected-assignment CURRENT_ASSIGNMENT_EVENT_ID \
  --agent codex --session SUCCESSOR_SESSION_ID --reason 'Hand off the remaining rollout'
```

Submit observations with `nexus delivery record FILE`, where FILE contains
`{"observations":[...]}`. Each observation has a unique `id`, source
`occurred_at`, and a `body` with an `event` tag. Use the emitted schema for the
complete fields. The MCP equivalent uses the `record` action. Exact replay under
the same ID is idempotent; changing accepted content under that ID is a conflict.

To hand off, read the latest assignment and append `shepherd_assigned` with its
expected ID, the successor, and why. Setting the agent to null releases the
responsibility. Do not infer a successful handoff from a stopped host session.

After a new push, read the PR's current head and `composition_event_id`, then
record `pull_request_composition_recorded` with `pull_request_id`, that exact
`head` GitRevision, `expected_composition`, the replacement `contribution_ids`,
and `reason`. The first composition version is the PR registration event ID.
Both head and version must still match. This updates included changes while
preserving the shepherd and earlier composition history; an older head is rejected.

## Watch PRs and environments

Provider observations use `pull_request_observed` with observation time, state,
base/head, and checks tied to their exact revision. A success on an older head is
not success for a newer head. A newer snapshot's `base_ref` and `head_ref` replace
the branch discovery entries when a PR is retargeted; earlier observations remain
in history. Register a configured watcher through `nexus delivery
watch FILE` or the typed `watch_registered` event; inspect the schema for supported
provider and interval fields. Watchers retain observations and failures rather
than approving, merging, or promoting code.

Inspect retained health with `nexus delivery watches`; stop a watch explicitly
with `nexus delivery watch-stop WATCH_ID`. GitHub credentials are optional for
public reads; private access uses the service's `NEXUS_DELIVERY_GITHUB_TOKEN_FILE`
pointing to a private credential file. Keep tokens out of records and command
arguments. Watch health and freshness are separate from PR/check outcomes:
inspect the last attempt, last success, next poll, and error. An enabled watch
does not establish freshness; provider failures retain earlier successful snapshots.

Use a rollout record to group source PRs/contributions and caller-defined
environments. Supply explicit `contribution_ids` to freeze that release's origin
scope across later source-PR edits. Missing manifests expose
`rollout_manifest_missing` and yield qualified PR candidates. Record each
deployment attempt with its environment, revision,
artifact identity where available, outcome, and related Work activity. Keep failed
attempts and rollbacks. An aggregate promotion must retain all source contribution
links; one shepherd does not become the author of the included implementation.

Finish a running or unknown attempt by recording a `deployment_finished` body:

```json
{"event":"deployment_finished","deployment_id":"staging-attempt-1","expected_outcome":"running","outcome":"failed"}
```

The expected outcome must match the retained state. The terminal outcome is
`succeeded`, `failed`, or `canceled`; the attempt's revision, artifact, and identity
stay unchanged. A retry is a new deployment attempt.

Use [work history](../nexus-work-history/SKILL.md) for canonical executions and
timing. Shared CI/release execution is recorded once with additional membership
links; delivery coordination should not copy or reparent every contributor's work.

## Route failures back to implementation

Ask Nexus using the failing revision and the narrowest recorded scope.

```sh
nexus delivery origin --repository acme/service --revision FULL_OBJECT_ID \
  --path src/cache.rs --line-start 20 --line-end 35
nexus delivery origin --repository acme/service --revision 01ab23c4 \
  --pull-request 42 --rollout ROLLOUT_ID --branch beta --feature timeline
nexus delivery origin --repository acme/service --fingerprint FULL_64_HEX_FINGERPRINT
```

A revision prefix requires a repository and at least four hexadecimal characters.
It expands only to a unique recorded commit. Add more digits if ambiguous.
Provider PR numbers use GitHub by default and require the repository. Use
`--pull-request-id` for an internal delivery ID. With a revision prefix,
provider PR number, branch, or `--fallback`, Nexus tries the revision or
fingerprint, PR composition, rollout manifest, branch, then feature. Other
combinations of legacy selectors keep their intersecting scope.

Read `matched_by`, qualification, rationale, and the activity groups before
choosing a recipient. A pinned contribution head can establish commit-level
candidates. Line-level attribution requires retained hunk evidence. A recorded
64-hex hunk fingerprint can survive changed commit identities. Multiple matches
remain visible with their own evidence.

For an evidenced failure, queue an addressed investigation from the same
origin selectors or from the unique location or feature on a recorded Code concern. See the [Code concerns guide](../nexus-code-concerns/SKILL.md).

```sh
nexus delivery remediate --repository acme/service --feature timeline \
  --title 'Investigate the rollout' --message 'Check the timeline replay' --evidence-file links.txt
nexus delivery remediate --concern CONCERN_ID \
  --title 'Investigate the regression' --message 'Review the linked evidence'
nexus delivery remediation-status REQUEST_ID
```

Provide one evidence reference per line in the evidence file or repeat
`--evidence`. Use `--agent` and `--session` to select an intended recipient when
the evidence is ambiguous. Specify the repository if the recipient has matches
in several. Use an explicit `--id` and replay the same request for a delivery
retry. The MCP equivalents are `remediate`, `remediation_status`, and
`acknowledge` on `nexus_delivery`.

The recipient's next supported prompt can surface a bounded preview.
`nexus delivery inbox` lists pending requests. After viewing the request, the
addressed agent can run `nexus delivery acknowledge REQUEST_ID` from the same
bound host identity. Status reports request state, verified receipt, explicit
presence evidence, and the recipient's last observed session activity separately.
Idle sessions can leave requests queued. Native chat messaging and resuming
old chats require the host's supported tools and caller authorization.

Use `remediation_transitioned` with expected state, reason, and the fixing
revision when known to retain progress, resolution, or declined and unreachable
outcomes. For an investigation needing detailed revision and delivery links,
submit a typed `remediation_requested` observation with those references and
the requested work, keeping the original rationale and constraints available.

## Check environment evidence before promotion

Record the required key names for the exact candidate revision and the
repository responsible for each key. The requirements JSON contains `id`,
`revision` (with `repository`, `object_format`, and full `commit`), and `requires` entries
with `repository` and `key`. The inventory JSON contains `id`, `repository`,
`environment`, `keys`, `complete`, and a UTC `expires_at`. Keep configuration
and secret values in their existing systems.

```sh
nexus delivery environment-record requirements requirements.json
nexus delivery environment-record inventory beta-inventory.json
nexus delivery environment-record advisory --environment beta \
  --title 'Database contention' --description 'Compare timings after the backlog clears' \
  --expires-at FUTURE_UTC_TIMESTAMP
nexus delivery environment --environment beta --pull-request-id PR_RECORD_ID
nexus delivery check-history --repository acme/service --check 'timeline replay'
```

Assess the registered head when the promotion PR opens or advances. To check
a standalone candidate, supply `--repository` and a full `--revision` instead
of `--pull-request-id`. An absent `requirements_id` means requirements are
unknown. A fresh inventory establishes listed keys as present. Omission from a
fresh, complete inventory establishes a missing key. Incomplete, expired, or
absent evidence leaves unresolved keys unknown. Read the environment alone before
benchmarking or replaying there to see active advisories.

Set `promotion_environment` on a PR watch to include readiness in
`nexus delivery watches` and its browser watch panel for the retained open
PR head. The environment read returns per-key evidence. Check history uses
the exact recorded check name and returns distinct PR IDs with failures on
retained snapshot heads. It defaults to seven days. Set `--since` for a different
UTC cutoff and check the `partial` flag before making coverage claims.

## Inspect and verify

The authenticated **Pull requests** view shows registered PRs, their shepherds,
rollouts, original contributions, exact revisions/checks, file/feature mappings,
deployment attempts, and remediation history. The list defaults to seven
days and offers 24 hours, 7, 14, and 30 days. It uses record update time.
A direct record link still opens its retained evidence. Use CLI/MCP for
registration and handoff. Follow observation times, stale reads, unknown
availability, candidate matches, and bounded history notices.
PR and contribution details show mapped file paths beside sampled current claims
and overlaps. Follow a session link to Coordination for its evidence. These live
advisories do not establish implementation authorship; no match means no overlap
in the observed window. Truncated coverage and failed refreshes remain visible.

Verify a representative workflow with two implementation agents and one shepherd,
an aggregate promotion, a failed environment attempt, provenance lookup, and a
remediation response. Check source identity and assignment history after one agent
stops. Unavailable Nexus capture must not stop or repeat the caller's workload.
Hand over concrete project, contribution, PR, rollout, deployment, and request IDs.
