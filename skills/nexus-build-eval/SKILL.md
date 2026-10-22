---
name: nexus-build-eval
description: Build or extend an evaluation and preserve its cases, grading definitions, motivation, and execution evidence in Nexus. Use for new eval suites and meaningful eval changes.
---

# Build an eval with a history

An eval is a measurement with a purpose. Start by identifying the decision it will support and the behavior it should distinguish. Keep the existing application entry point, runner, and grader where they work; Nexus stores the evidence and history while the caller executes the evaluation.

Read [the Nexus record and evidence contract](../nexus-eval-history/references/nexus-evals.md) before recording data. Use `nexus-eval-history` to inspect related suites, failed attempts, and prior change rationales before designing a new case set.

## Design from the reason

Capture the motivating observation before changing the eval: a production failure, user requirement, suspected regression, coverage gap, or audit finding. Link the actual supporting records or artifacts. Explain what existing coverage missed, what a new case should reveal, and how the result could change a decision. Preserve an uncertain hypothesis as uncertain; do not invent an incident or human approval.

For an existing suite, record a new revision with its predecessor and rationale. Include additions, removals, corrections, grader edits, changed weights, and split changes. A reason such as “add edge cases” does not explain why those cases matter. Separate expanding the measurement from improving the system measured by it.

Choose realistic inputs and record their origin and any transformation: human-authored, production-derived, synthetic, or model-generated, including the generator where known. Prefer production failures and representative examples over cases inferred only from the current prompt. Distinguish expected answers from reference outputs, rubrics, and environment end-state assertions.

## Build the measurement

- Exercise the actual application path, including context assembly and tool routing. Stub only dependencies that cannot be exercised safely or reproducibly; record the substitutions.
- Grade the outcome the user cares about. For an agent editing files or state, inspect the resulting environment. Use model judgment for properties that programmatic checks cannot establish.
- Keep quality dimensions and cost/latency separate. Record the denominator, aggregation, exclusions, and direction of each metric.
- Calibrate with known good and known bad outputs and a few real examples. Resolve disagreements about the cases or rubric before spending a full run on an unsettled measurement.
- Keep reference answers and held-out evidence outside the model-under-test's reachable context. Record split membership and any known prior exposure.
- Preserve cases and grading definitions as immutable revisions. A later correction creates a new version and, where useful, another scoring pass over the original outputs.

Resolve consequential uncertainty with the user, especially what success means or whether inputs represent real use. Reuse clear authorization and established product requirements; avoid an approval ritual for routine implementation choices. Obtain authorization before paid or externally mutating execution when it is not already in scope.

## Capture and hand over

Before the run, record the suite revision, candidate configuration, repository revision, grader revision, fixtures, and available prompts/tools/skills. During execution, record each attempt and each captured call with its evidence from that same execution. Separate refusals, timeouts, infrastructure errors, truncated outputs, and genuine task failures. Count retries and billed failed attempts.

Audit the pilot with `nexus-eval-audit`, then complete the authorized run. Derive summaries from recorded per-case observations, qualify uncertainty, and link the baseline and evidence. Explain how another agent can reproduce or re-score the run, including any unavailable external state. Do not claim complete capture when the harness exposes only partial context.

The history should answer: why this case exists, which definition was evaluated, what the model could see and use, what actually happened, and why the recorded result supports the decision.

## Sources

Original Nexus workflow informed by Anthropic's [eval construction guide](https://github.com/anthropics/skills/blob/dbd4588f9e1033efb41dad4bef2f7947c8993d44/skills/claude-api/shared/evals/build-eval.md) and [eval audit guide](https://github.com/anthropics/skills/blob/dbd4588f9e1033efb41dad4bef2f7947c8993d44/skills/claude-api/shared/evals/eval-audit.md). Provider and execution framework remain the user's choice.
