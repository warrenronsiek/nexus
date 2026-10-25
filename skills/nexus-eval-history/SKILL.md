---
name: nexus-eval-history
description: Retrieve and explain Nexus evaluation history, including why cases changed, what configurations were tried, and which evidence supports scores or decisions.
---

# Read the reasons behind eval results

Read [the Nexus record and evidence contract](references/nexus-evals.md) for available commands, record structure, evidence capture, and import/export behavior.

For a human overview, open `nexus tui` or Pi `/nexus` and select Evals. The default view groups suites, targets, repositories and model compositions; Enter shows result trends, `[ ]` changes the metric, and `b` opens the secondary immutable record browser. Metric revisions and assessment scopes remain separate. Charts summarize captured evidence and disclose partial history; they do not establish which result is better.

Start with the user's question: a behavior, repository, model, run, case, or decision. List with the narrowest useful filters, follow pagination, then inspect records and their rationale links. Use `why` to traverse motivating evidence; the attached rationale explains a change, while a later summary is an interpretation of it.

## Reconstruct the chain

Trace from the decision or score to the scoring pass, original attempt and output, candidate, suite/case/grader revisions, and their reasons for changing. Inspect previous revisions and rejected candidates where they explain the current choice. Resolve referenced artifacts as needed rather than loading every transcript into context.

For “what did the model have available,” inspect the call's ordered instructions, tool/skill inventory, file evidence, request parameters, served model, and capture-completeness notes. Distinguish configured, exposed, invoked, and completed capabilities. Absence from a partial capture cannot establish absence at execution time.

For “what worked,” compare only compatible measurements, and identify any differences in cases, grading, metric definitions, units, calculation sources, formats, populations, filters, retries, environment, or holdout exposure. Values may be exact decimal text, categorical labels, JSON/CSV, distributions, or artifacts; inspect their definitions before interpreting them. Nexus preserves results and the caller's comparison claim without computing an objective or deciding which result is better. Keep a candidate's repository revision and model configuration together. For routers or multi-agent systems, follow the composition and actual call lineage.

For “why was this eval added,” retrieve the revision's own rationale and its triggering incident, requirement, gap, or observation. Do not infer intent from a filename, commit message, or a model-written summary when the original motivation is missing. Say what is unknown and preserve the source's uncertainty.

For “what existed at that time,” distinguish occurrence time, Nexus receipt time, and a selection's effective time. Follow selection and parent links across all relevant pages. A backdated import can describe an earlier event while becoming known to Nexus later. Parallel revision branches do not identify a selected version by themselves.

## Write back only when requested

Inspection is read-only. If asked to correct, extend, import, or annotate history, append the appropriate record and preserve prior observations. Include the reason and relevant links. Do not fabricate old timestamps, approvals, or a complete historical context from incomplete logs.

Return a concise answer with record IDs, the relevant revision boundaries, and evidence references. A useful handoff explains the result, why it exists, what was tried, and what the evidence cannot establish.

## Sources

Original Nexus workflow informed by the evidence and adapter practices in Anthropic's [eval report contract](https://github.com/anthropics/skills/blob/dbd4588f9e1033efb41dad4bef2f7947c8993d44/skills/claude-api/shared/evals/report/SCHEMA.md). Nexus history remains the source of recorded facts; narrative summaries are derived views.
