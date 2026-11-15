---
name: nexus-pr-management
description: Register and discover PR shepherds, trace released changes to their original implementation agents, observe rollout progress, and retain evidence-backed remediation handoffs in Nexus.
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
source revisions, Git operation, verification, and changed hunks. Preserve
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

Ask Nexus using the failing deployed revision and the narrowest available scope:

```sh
nexus delivery origin --repository acme/service --revision FULL_OBJECT_ID \
  --path src/cache.rs --line-start 20 --line-end 35
nexus delivery origin --repository acme/service --feature cache
nexus delivery origin --repository acme/service --fingerprint FULL_64_HEX_FINGERPRINT
```

The response returns original contributors, rationale, constraints, source and
result mappings, known work/contact references, qualification, and evidence gaps.
Exact, candidate, ambiguous, and unknown results are meaningful. A feature-only
query provides leads; verify them against the failure before assigning blame.
Repository-qualified hunk fingerprints can match retained content across changed
commit identities. Use the recorded 64-hex content fingerprint; identical content
attributed to multiple contributors remains ambiguous rather than selecting one
author.

For an evidenced failure, record `remediation_requested` with its recipient,
contribution/PR/rollout/deployment links, failing revision, evidence, and the
specific requested investigation. Link a Code concern when a defect is known;
use [Code concerns](../nexus-code-concerns/SKILL.md) for its encounters/remedies.
Include the original rationale and constraints in the handoff context instead of
asking the next agent to infer intent from the final code blob.

Check `nexus delivery inbox` for pending requests. The recipient appends
`remediation_transitioned` with expected/current state, reason, and a fixing
revision when known. A request, acknowledgment, in-progress attempt, and verified
resolution are different observations. Preserve declined or unreachable states.

## Contact and availability

Use actual host/thread contact references and `agent_presence_observed` evidence.
The registration is durable; reachability is an observation with its own time.
No presence record means unknown. A host may be stopped while its contribution
remains attributable and its remediation request remains pending.

Nexus retains routing references and an inbox. Sending a native host message or
resuming an old chat requires that host's supported tools and caller authorization.
Do not claim that recording a remediation delivered it, woke the original agent,
or made the agent accept the task. When a host cannot be reached, return the
context packet and record an explicit successor or unreachable outcome.

## Inspect and verify

The authenticated **Pull requests** view shows registered PRs, their shepherds,
rollouts, original contributions, exact revisions/checks, file/feature mappings,
deployment attempts, and remediation history. All browser reads are read-only;
use CLI/MCP for registration and handoff. Follow observation times, stale reads,
unknown availability, candidate matches, and bounded history notices.
PR and contribution details show mapped file paths beside sampled current claims
and overlaps. Follow a session link to Coordination for its evidence. These live
advisories do not establish implementation authorship; no match means no overlap
in the observed window. Truncated coverage and failed refreshes remain visible.

Verify a representative workflow with two implementation agents and one shepherd,
an aggregate promotion, a failed environment attempt, provenance lookup, and a
remediation response. Check source identity and assignment history after one agent
stops. Unavailable Nexus capture must not stop or repeat the caller's workload.
Hand over concrete project, contribution, PR, rollout, deployment, and request IDs.
