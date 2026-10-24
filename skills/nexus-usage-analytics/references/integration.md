# Host-adapter integration

Prefer the companion and generated host hooks. For a new adapter, discover the
installed `nexus_pre_tool_use`, `nexus_post_tool_use`,
`nexus_post_tool_failure`, `nexus_skill_use`, and `nexus_analytics` MCP schemas.
This reference explains the identities and evidence; schemas remain authoritative.

## Invocation and skill evidence

Use the host's stable session and invocation IDs, concrete tool name, agent
namespace, actual outcome, and available parent/turn/model context. Start and
completion share a `tool_use_id`; repeated delivery is an upsert, while another
execution has a new ID. A completion without an observed start is still useful.
Forward supported inputs only through the lifecycle boundary; never create a
parallel analytics log of raw arguments or results. Capture must be bounded and
fail open without replacing output or repeating the workload.

Explicit skill activation uses `nexus_skill_use` with `session_id`,
`invocation_id`, `skill_name`, and `evidence`. Evidence values are `native_hook`,
`explicit_invocation`, `instruction_read`, and `asset_execution`. Use the event
actually observed rather than claiming a successful read is a native activation.
Available context includes `project_root`, `agent`, `turn_id`, `model`, and
`actor`. Stronger same-turn evidence can promote an existing skill observation;
do not manufacture multiple activations for one read/activation sequence.

## Inventory snapshot

Call `nexus_analytics` with this shape, replacing example identities and resource
definitions with the host's actual values:

```json
{
  "action":"inventory",
  "session_id":"host-session-42",
  "agent":"custom-host",
  "project_root":"/absolute/path/to/checkout",
  "scope":"host-tools",
  "complete":true,
  "resources":[
    {
      "name":"read_file",
      "kind":"tool",
      "origin":"host_tool",
      "exposure":"exposed",
      "description":"Read a repository file",
      "version":"definition-fingerprint-v1",
      "activities":[]
    }
  ]
}
```

`scope` is a bounded slug identifying the inventory source. The snapshot accepts
at most 256 resources. `complete` defaults true, so explicitly set false for a
partial/truncated view. Do not split one incomplete scope into multiple complete
claims or infer resources the SDK did not expose. Use a stable scope for its
updates and report changes; prior session exposure remains historical evidence.

Resource kinds are `tool`, `script`, `cli`, `inline`, and `skill`. Origins are
`host_tool`, `repository`, `standard_cli`, `agent_authored`, `unknown`, and
`internal`. Exposure is `declared`, `discoverable`, or `exposed`. Repository script
names must be relative and cannot contain parent traversal. Definition versions
should change with the actual definition; do not use a random value every call.
`activities` contains existing category IDs when genuinely known, at most eight.
Descriptions are bounded safe metadata, not complete private tool prompts.

## Classification and retrieval

Ordinary adapters should rely on companion enrichment. Direct HTTP lifecycle
clients retain facts but do not automatically get checkout-side script inference
or classification. If implementing an enrichment adapter, use the current typed
`reserve_classification`/`classify`/`classification_failed` contract: targets refer
to actual recorded actions/resources and stable pattern fingerprints, reservation
must be granted, and reports must match that subject/version. Never assign labels
to invented calls, bypass the budget, or persist the transient model input.
Shared-service deployment does not require enabling provider subprocesses there.

`summary`, `catalog`, `opportunities`, `examples`, and `categories` are read
actions. Catalog takes `offset` and `limit` (default 32, maximum 64). Examples
take `offset` and optional `category_id` or `resource_key`; implementation pages
contain at most 32 records. Use returned pagination/coverage metadata and retain
the requested project scope on every page.

Taxonomy maintenance uses `rename_category` (`id`, `name`, `definition`) and
`merge_category` (`source`, `target`). These change shared meaning, so use them
when the actual task calls for taxonomy maintenance, after inspecting categories
and examples. Merged IDs redirect later evidence; category edits do not rewrite
the original invocation facts. These actions are not necessary for ordinary
capture or for reading opportunity suggestions.
