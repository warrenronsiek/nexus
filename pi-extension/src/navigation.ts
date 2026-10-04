// @feature observability-ui
// @spec docs/features/observability-ui.md
import type {
  ClaimRecord,
  ConflictRecord,
  EventRecord,
  SessionRecord,
  Snapshot,
} from "./domain.ts";

export type Section = "projects" | "conflicts" | "sessions" | "claims" | "events";
export type RecordSection = Exclude<Section, "projects">;
export type Page =
  | { kind: "overview" }
  | { kind: "list"; section: Section }
  | { kind: "detail"; section: RecordSection; recordKey: string };

export interface Entry {
  value: string;
  label: string;
  description?: string;
}

export type Selection =
  | { kind: "navigate"; page: Page }
  | { kind: "scope"; projectId: string | null };

function shortPath(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts.slice(-3).join("/") || path;
}

function shortId(value: string): string {
  return value.length > 12 ? `${value.slice(0, 12)}…` : value;
}

function localTime(value: string): string {
  const time = new Date(value);
  return Number.isNaN(time.valueOf()) ? value : time.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function overviewEntries(snapshot: Snapshot): Entry[] {
  const { counts, project_id: projectId } = snapshot.dashboard;
  const scope = projectId === null ? "All projects" : shortId(projectId);
  return [
    { value: "section:projects", label: "Project scope", description: scope },
    { value: "section:conflicts", label: `Conflicts  ${counts.open_conflicts}`, description: "Open overlaps" },
    { value: "section:sessions", label: `Sessions   ${counts.active_sessions}`, description: "Active agents" },
    { value: "section:claims", label: `Claims     ${counts.active_claims}`, description: "Advisory path ownership" },
    { value: "section:events", label: `Events     ${counts.events}`, description: "Recent activity" },
  ];
}

function conflictEntry(record: ConflictRecord): Entry {
  return {
    value: `record:${record.id}`,
    label: `${record.severity.toUpperCase()}  ${shortPath(record.path)}`,
    description: record.message,
  };
}

function sessionEntry(record: SessionRecord): Entry {
  return {
    value: `record:${record.session_id}`,
    label: `${record.agent}  ${shortId(record.session_id)}`,
    description: record.task_summary ?? record.worktree ?? record.status,
  };
}

function claimEntry(record: ClaimRecord): Entry {
  const range = record.line_start === null ? "" : `:${record.line_start}${record.line_end === null ? "" : `-${record.line_end}`}`;
  return {
    value: `record:${record.id}`,
    label: `${record.operation.toUpperCase()}  ${shortPath(record.path)}${range}`,
    description: `${shortId(record.session_id)} · ${record.state}`,
  };
}

function eventEntry(record: EventRecord): Entry {
  return {
    value: `record:${record.id}`,
    label: record.kind,
    description: `${record.session_id ? shortId(record.session_id) : "system"} · ${localTime(record.created_at)}`,
  };
}

function emptyEntry(section: Section): Entry[] {
  return [{ value: "noop", label: `No ${section} in this scope` }];
}

export function entriesFor(snapshot: Snapshot, page: Exclude<Page, { kind: "detail" }>): Entry[] {
  if (page.kind === "overview") return overviewEntries(snapshot);
  if (page.section === "projects") {
    return [
      { value: "project:all", label: "All projects", description: "Machine-wide activity" },
      ...snapshot.projects.map((project) => ({
        value: `project:${project.project_id}`,
        label: shortId(project.project_id),
        description: `${project.agents.join(", ") || "no active agents"} · ${project.worktrees[0] ?? "unknown worktree"}`,
      })),
    ];
  }
  const entries = {
    conflicts: snapshot.dashboard.conflicts.items.map(conflictEntry),
    sessions: snapshot.dashboard.sessions.items.map(sessionEntry),
    claims: snapshot.dashboard.claims.items.map(claimEntry),
    events: snapshot.dashboard.events.items.map(eventEntry),
  }[page.section];
  return entries.length === 0 ? emptyEntry(page.section) : entries;
}

function recordExists(snapshot: Snapshot, section: RecordSection, key: string): boolean {
  return recordFor(snapshot, { kind: "detail", section, recordKey: key }) !== undefined;
}

export function selectEntry(snapshot: Snapshot, page: Page, value: string): Selection {
  if (value === "noop") return { kind: "navigate", page };
  if (page.kind === "overview" && value.startsWith("section:")) {
    const section = value.slice("section:".length) as Section;
    return { kind: "navigate", page: { kind: "list", section } };
  }
  if (page.kind === "list" && page.section === "projects" && value.startsWith("project:")) {
    const projectId = value.slice("project:".length);
    return { kind: "scope", projectId: projectId === "all" ? null : projectId };
  }
  if (page.kind === "list" && page.section !== "projects" && value.startsWith("record:")) {
    const recordKey = value.slice("record:".length);
    if (recordExists(snapshot, page.section, recordKey)) {
      return { kind: "navigate", page: { kind: "detail", section: page.section, recordKey } };
    }
  }
  return { kind: "navigate", page };
}

export function backPage(page: Page): Page | null {
  if (page.kind === "overview") return null;
  if (page.kind === "detail") return { kind: "list", section: page.section };
  return { kind: "overview" };
}

export function pageTitle(page: Page): string {
  if (page.kind === "overview") return "Nexus";
  if (page.kind === "detail") return `${page.section.slice(0, -1)} detail`;
  return page.section === "projects" ? "Project scope" : page.section[0].toUpperCase() + page.section.slice(1);
}

function recordFor(snapshot: Snapshot, page: Extract<Page, { kind: "detail" }>): unknown {
  const key = page.recordKey;
  if (page.section === "events") return snapshot.dashboard.events.items.find((record) => String(record.id) === key);
  if (page.section === "sessions") return snapshot.dashboard.sessions.items.find((record) => record.session_id === key);
  return snapshot.dashboard[page.section].items.find((record) => record.id === key);
}

export function detailText(snapshot: Snapshot, page: Extract<Page, { kind: "detail" }>): string {
  const record = recordFor(snapshot, page);
  return record === undefined ? "This record is no longer present in the current window." : JSON.stringify(record, null, 2);
}

export function scopeLabel(snapshot: Snapshot): string {
  const selected = snapshot.dashboard.project_id;
  if (selected === null) return "all projects";
  const project = snapshot.projects.find((candidate) => candidate.project_id === selected);
  return project?.worktrees[0] ?? shortId(selected);
}
