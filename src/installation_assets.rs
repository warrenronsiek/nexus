// @feature installation
// @feature code-concerns
// @feature sos
// @feature usage-analytics
// @feature evals
// @feature work-history
// @spec docs/features/installation.md
// @spec docs/features/code-concerns.md
// @spec docs/features/sos.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/evals.md
// @spec docs/features/work-history.md
// Bundled source assets form one installation manifest shared by both hosts.

pub(super) const SKILL_NAMES: [&str; 18] = [
    "code-architect",
    "code-deletion",
    "tdd",
    "nexus",
    "nexus-setup",
    "nexus-coordination",
    "nexus-agent-memory",
    "nexus-papercuts",
    "nexus-sos",
    "nexus-code-concerns",
    "nexus-usage-analytics",
    "nexus-observability",
    "nexus-development",
    "nexus-build-eval",
    "nexus-eval-audit",
    "nexus-hillclimb",
    "nexus-eval-history",
    "nexus-work-history",
];
pub(super) const SKILL_FILES: [(&str, &str); 28] = [
    (
        "code-architect/SKILL.md",
        include_str!("../skills/code-architect/SKILL.md"),
    ),
    (
        "code-architect/references/feature-mapping.md",
        include_str!("../skills/code-architect/references/feature-mapping.md"),
    ),
    (
        "code-deletion/SKILL.md",
        include_str!("../skills/code-deletion/SKILL.md"),
    ),
    ("nexus/SKILL.md", include_str!("../skills/nexus/SKILL.md")),
    (
        "nexus-code-concerns/SKILL.md",
        include_str!("../skills/nexus-code-concerns/SKILL.md"),
    ),
    (
        "nexus-agent-memory/SKILL.md",
        include_str!("../skills/nexus-agent-memory/SKILL.md"),
    ),
    (
        "nexus-build-eval/SKILL.md",
        include_str!("../skills/nexus-build-eval/SKILL.md"),
    ),
    (
        "nexus-coordination/SKILL.md",
        include_str!("../skills/nexus-coordination/SKILL.md"),
    ),
    (
        "nexus-development/SKILL.md",
        include_str!("../skills/nexus-development/SKILL.md"),
    ),
    (
        "nexus-development/references/persistence-validation.md",
        include_str!("../skills/nexus-development/references/persistence-validation.md"),
    ),
    (
        "nexus-development/references/source-structure.md",
        include_str!("../skills/nexus-development/references/source-structure.md"),
    ),
    (
        "nexus-eval-audit/SKILL.md",
        include_str!("../skills/nexus-eval-audit/SKILL.md"),
    ),
    (
        "nexus-eval-history/SKILL.md",
        include_str!("../skills/nexus-eval-history/SKILL.md"),
    ),
    (
        "nexus-eval-history/references/nexus-evals.md",
        include_str!("../skills/nexus-eval-history/references/nexus-evals.md"),
    ),
    (
        "nexus-hillclimb/SKILL.md",
        include_str!("../skills/nexus-hillclimb/SKILL.md"),
    ),
    (
        "nexus-observability/SKILL.md",
        include_str!("../skills/nexus-observability/SKILL.md"),
    ),
    (
        "nexus-papercuts/SKILL.md",
        include_str!("../skills/nexus-papercuts/SKILL.md"),
    ),
    (
        "nexus-setup/SKILL.md",
        include_str!("../skills/nexus-setup/SKILL.md"),
    ),
    (
        "nexus-setup/references/integration.md",
        include_str!("../skills/nexus-setup/references/integration.md"),
    ),
    (
        "nexus-sos/SKILL.md",
        include_str!("../skills/nexus-sos/SKILL.md"),
    ),
    (
        "nexus-usage-analytics/SKILL.md",
        include_str!("../skills/nexus-usage-analytics/SKILL.md"),
    ),
    (
        "nexus-usage-analytics/references/integration.md",
        include_str!("../skills/nexus-usage-analytics/references/integration.md"),
    ),
    (
        "nexus-work-history/SKILL.md",
        include_str!("../skills/nexus-work-history/SKILL.md"),
    ),
    (
        "nexus-work-history/references/integration.md",
        include_str!("../skills/nexus-work-history/references/integration.md"),
    ),
    (
        "nexus-work-history/references/host-adapters.md",
        include_str!("../skills/nexus-work-history/references/host-adapters.md"),
    ),
    ("tdd/SKILL.md", include_str!("../skills/tdd/SKILL.md")),
    (
        "nexus-work-history/scripts/work-capture.sh",
        include_str!("../scripts/work-capture.sh"),
    ),
    (
        "nexus-work-history/scripts/nexus-work-github.py",
        include_str!("../scripts/nexus-work-github.py"),
    ),
];
pub(super) const PI_EXTENSION_FILES: [(&str, &str); 6] = [
    ("capture.ts", include_str!("../pi-extension/src/capture.ts")),
    (
        "host-context.ts",
        include_str!("../pi-extension/src/host-context.ts"),
    ),
    ("index.ts", include_str!("../pi-extension/src/index.ts")),
    (
        "inventory-capture.ts",
        include_str!("../pi-extension/src/inventory-capture.ts"),
    ),
    (
        "mcp-client.ts",
        include_str!("../pi-extension/src/mcp-client.ts"),
    ),
    (
        "memory-context.ts",
        include_str!("../pi-extension/src/memory-context.ts"),
    ),
];

// Remove only modules installed by the retired Nexus terminal interface.
pub(super) const RETIRED_PI_UI_FILES: [&str; 51] = [
    "dashboard-options.ts",
    "concerns.ts",
    "concerns-pane.ts",
    "concerns-state.ts",
    "concerns-requests.ts",
    "concerns-view.ts",
    "concerns-evidence.ts",
    "charts.ts",
    "work-waterfall.ts",
    "work-activity-style.ts",
    "work-waterfall-metadata.ts",
    "work-interval.ts",
    "work-dashboard.ts",
    "work-dashboard-wire.ts",
    "evals-dashboard.ts",
    "evals-dashboard-view.ts",
    "history-pane.ts",
    "text-filter.ts",
    "wire-values.ts",
    "work-navigation.ts",
    "work-requests.ts",
    "analytics-data.ts",
    "analytics-requests.ts",
    "analytics-view.ts",
    "analytics.ts",
    "papercuts-pane.ts",
    "papercuts.ts",
    "work-pane.ts",
    "work-state.ts",
    "work-view.ts",
    "work.ts",
    "client.ts",
    "component.ts",
    "connection.ts",
    "display.ts",
    "domain.ts",
    "evals.ts",
    "evals-pane.ts",
    "evals-view.ts",
    "evals-state.ts",
    "evals-requests.ts",
    "memory-pane.ts",
    "memory-prompts.ts",
    "memory.ts",
    "navigation.ts",
    "polling.ts",
    "sos-pane.ts",
    "sos.ts",
    "theme.ts",
    "usage-pane.ts",
    "usage.ts",
];
