---
name: nexus-eval-audit
description: Audit an evaluation's measurement quality, historical provenance, and reproducibility using Nexus records and run artifacts. Use before trusting comparisons or diagnosing surprising eval scores.
---

# Audit an eval and its evidence

Read [the Nexus record and evidence contract](../nexus-eval-history/references/nexus-evals.md), then identify the exact suite revision, candidate, execution, and scoring pass under discussion. Inspect an actual recent attempt or run a bounded authorized smoke test. A README and aggregate score alone cannot establish that an eval works.

Record findings against the evidence they concern. Distinguish observed defects, unsupported assumptions, and deliberate tradeoffs. Explain the effect on the decision and a concrete correction. Do not manufacture findings to fill a checklist.

## Inspect the measurement

- **Purpose and cases:** Can the recorded motivation explain each added or changed case? Are source and reference provenance known? Check representative coverage, duplicate clusters, label validity, stale facts, and accidental answer leakage. Sample by meaningful strata rather than only memorable failures.
- **Execution:** Does the runner invoke the real application? Verify isolated initial state, fixtures, seeds, environment, dependency versions, model actually served, routing, tools, and substitutions. Compare declared configuration with observed call evidence.
- **Grading:** Inspect the exact grader revision and its inputs. Test known good, empty, and plausible wrong outputs. For judges, check calibration, structured results, ordering bias, unsupported certainty, and whether candidate text could steer the judge. A grading crash is not a capability failure.
- **Accounting:** Confirm every attempted request is represented, including retries and billed failures. Distinguish task quality, refusal, truncation, timeout, provider failure, tool failure, and missing evidence. Check timing boundaries and cost basis rather than assuming a zero means free or fast.
- **Comparisons:** Recompute the headline from recorded observations. Confirm case revisions, split membership, grading revision, aggregation, exclusions, and environment are compatible. Check variance and whether the measured effect exceeds the uncertainty relevant to the decision.
- **Evidence:** Follow a score to its original output, call context, artifacts, grader, and change motivation. Confirm hashes and Git paths resolve to the intended contents. Mark missing files, redactions, declared-only tool inventories, and inaccessible references explicitly.

For an agentic eval, inspect the resulting files or state and the available tools at call time. A transcript claiming success is evidence of a claim. A tool declared in a configuration is evidence of a declaration. Neither proves that a capability executed or that the intended state changed.

## Repair without erasing the finding

Record a correction as a new definition or scoring revision with its reason and evidence. Re-score saved outputs where the correction permits it; re-execute only when necessary. Preserve original scores, report which cases and rankings changed, and qualify older conclusions that depended on the defect.

If an external dependency changed or an artifact cannot be recovered, preserve that limitation rather than reconstructing an allegedly original file from memory. If the same observed outputs receive a new interpretation, retain both interpretations.

Report the few issues that affect the decision first, with Nexus record IDs and artifact references. Say when the evidence supports the comparison. Keep optional improvements separate from issues that invalidate the result.

## Sources

Original Nexus workflow informed by Anthropic's [eval health checklist](https://github.com/anthropics/skills/blob/dbd4588f9e1033efb41dad4bef2f7947c8993d44/skills/claude-api/shared/evals/eval-audit.md). Historical correction and evidence requirements belong to Nexus.
