---
name: nexus-interface-design
description: Design or review the Nexus developer observability interface using the pinned Leonxlnx taste-skill and Nexus evidence constraints.
---

# Nexus interface design

Read [the upstream taste-skill](../../../vendor/taste-skill/SKILL.md), then its
[app-screen guidance](../../../vendor/taste-skill/references/app-screens.md).
Resolve these links from this skill directory. Run its scripts from the repository
root. The vendored skill and license are pinned in `vendor/taste-skill/UPSTREAM.md`.

Read `docs/features/observability-ui.md` and `web/DESIGN.md` before editing.
Nexus is a developer metrics console in the Grafana/SigNoz family. Preserve the
chosen data-terminal direction, self-hosted Public Sans / IBM Plex Mono, Lucide
SVG icons, visible labels, neutral surfaces, and the shared chart tokens.
Use the branching session waterfall as the product's signature visualization.

Keep Elm responsible for the interface and D3 responsible only for charts. Never
invent observations, optimization directions, time windows, comparison deltas,
or controls without supported reads. Preserve source precision and uncertainty.
Authentication and all nine evidence surfaces remain read-only. Do not add
marketing heroes, gradients, glowing surfaces, placeholder glyphs, or icon tiles.

Check real screens at desktop, tablet, and phone widths, including long identifiers,
empty/error states, selected waterfall metadata, and exact eval values. Follow
`nexus-development` for behavior tests, the canonical gate, and commit review.
Use the approved computer-use tool for interactive QA if available; do not run
the upstream raw-CDP inspector when that conflicts with host tool restrictions.
