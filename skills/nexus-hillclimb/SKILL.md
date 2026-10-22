---
name: nexus-hillclimb
description: Improve a model-backed system against an existing eval while recording hypotheses, candidate changes, execution evidence, and adoption decisions in Nexus.
---

# Improve a system against recorded evidence

Read [the Nexus record and evidence contract](../nexus-eval-history/references/nexus-evals.md). Inspect the existing evaluation and earlier experiments with `nexus-eval-history`; use `nexus-eval-audit` when its validity is uncertain. Without a runnable trustworthy measurement, use `nexus-build-eval` first.

Nexus is the historical repository. The caller owns the harness, provider calls, sandbox, and optimization loop. Recording a candidate or experiment does not execute it.

## Establish the experiment

Define the target metric, direction, guardrails, allowed changes, stopping condition, and authorized execution budget. Reuse prior user decisions. Resolve a missing consequential constraint before the dependent action; avoid redundant questions when the scope is already clear.

Freeze a baseline against an exact suite and grader revision. Keep split membership stable and restrict change-proposing agents to the intended development evidence. Track which cases informed proposals. Repeated selection against a validation set consumes its independence; reserve a final untouched confirmation set when the decision requires it.

Record the repository/model configuration pair for each candidate. A prompt that helps one model may not transfer to another. A composed router or multi-agent candidate should name its component candidates and routing configuration rather than flattening them into one model label. Preserve the actual per-call models and context observed during execution.

## Run hypothesis-driven rounds

1. Inspect development failures and earlier rejected attempts. Record the proposed explanation, linked evidence, expected effect, expected regressions, and what would falsify it. Estimate whether the eval can distinguish the effect from its noise.
2. Create a new candidate with the exact changed instructions, tools, configuration, or code. Prefer one causal idea per round; document a bundled coverage intervention as a bundle. Snapshot changes using durable Git references or verified artifacts.
3. Execute the authorized harness against the fixed measurement and record attempts, outputs, call manifests, usage, failures, and grading evidence. Keep the original observations when another judge or scoring rule is applied later.
4. Compare compatible records and record the decision: adopt, reject, defer, or inconclusive. Attach the reason and supporting comparison, including guardrail movement. An unexplained positive delta is a lead to investigate.

Do not silently change cases, labels, weights, grader, filters, or denominators to improve a candidate's apparent result. Record measurement changes separately, explain why, and establish which old and new results remain comparable. Where useful, re-score the stored outputs under both grading revisions.

When progress stalls, distinguish missing capability, unreachable context, tool or harness defects, grader disagreement, and variance before adding instructions. Retain negative results and discarded candidates so another session can avoid repeating them.

## Finish with a defensible decision

Confirm the selected candidate on the appropriate held-out evidence and, if necessary, against a fresh unchanged control. State absolute quality/cost/latency, uncertainty, and comparison limits. Preserve the full search history and the final adoption rationale. Explain what remains untested, especially transfers to other models, repositories, or router compositions.

Apply or release the winner only within the user's authorized scope. A recorded recommendation does not imply approval for deployment or more model spend.

## Sources

Original Nexus workflow informed by Anthropic's [hillclimb guide](https://github.com/anthropics/skills/blob/dbd4588f9e1033efb41dad4bef2f7947c8993d44/skills/claude-api/shared/evals/eval-hillclimb.md) and [cost search guide](https://github.com/anthropics/skills/blob/dbd4588f9e1033efb41dad4bef2f7947c8993d44/skills/claude-api/shared/evals/cost-hillclimb.md). Nexus retains historical revisions instead of replacing prior grades.
