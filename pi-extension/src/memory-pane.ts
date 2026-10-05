// @feature agent-memory
// @feature observability-ui
// @spec docs/features/agent-memory.md
// @spec docs/features/observability-ui.md
import type { DashboardTheme as Theme } from "./theme.ts";
import {
  Key,
  matchesKey,
  truncateToWidth,
  wrapTextWithAnsi,
  type Component,
  type TUI,
} from "@earendil-works/pi-tui";
import type {
  MemoryApi,
  MemoryEntry,
  MemoryNode,
  MemorySummary,
  MemoryView,
} from "./memory.ts";

export interface MemoryInteractionPrompts {
  searchRegex(): Promise<string | undefined>;
  newMemory(): Promise<{
    scope: "global" | "project";
    content: string;
  } | undefined>;
  confirmInvalidation(summary: MemorySummary): Promise<boolean>;
}

type MemoryMode = "root" | "expanded" | "search" | "detail";
type CurrentAction = () => boolean;

const MAX_VISIBLE_NODES = 12;
const MAX_VISIBLE_DETAIL_LINES = 12;
const MAX_VISIBLE_OMISSIONS = 3;

function scopeLabel(node: MemoryNode): string {
  return node.scope.scope === "global" ? "global" : "project";
}

function nodeLine(node: MemoryNode): string {
  if (node.kind === "raw") {
    return `raw #${node.ordinal} · ${scopeLabel(node)} · ${node.content}`;
  }
  return `summary #${node.start_ordinal}-${node.end_ordinal} · level ${node.level} · provider ${node.provider} · ${node.content}`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function windowStart(total: number, selected: number, maximum: number): number {
  const centered = selected - Math.floor(maximum / 2);
  return Math.max(0, Math.min(centered, Math.max(0, total - maximum)));
}

function formatSummaryCount(count: number): string {
  return `${count} ${count === 1 ? "summary" : "summaries"}`;
}

interface MemoryRenderState {
  view: MemoryView | undefined;
  nodes: MemoryNode[];
  selectedIndex: number;
  mode: MemoryMode;
  detail: MemoryEntry | undefined;
  detailScroll: number;
  status: string;
}

interface MutableMemoryPaneState extends MemoryRenderState {
  active: boolean;
  generation: number;
  detailLineCount: number;
  actionSequence: number;
  activeAction: number | undefined;
}

interface MemoryPaneDependencies {
  api: MemoryApi;
  prompts: MemoryInteractionPrompts;
  changed(): void;
}

interface RunningMemoryAction {
  generation: number;
  action: number;
}

interface MemoryRenderResult {
  lines: string[];
  detailLineCount: number;
}

function renderMemoryPane(theme: Theme, width: number, state: MemoryRenderState): MemoryRenderResult {
  const contentWidth = Math.max(1, width - 2);
  const health = memoryHealthLines(theme, state.view, state.status);
  const content = state.detail === undefined
    ? renderMemoryNodes(theme, contentWidth, state)
    : renderMemoryDetail(theme, contentWidth, state.detail, state.detailScroll);
  return {
    lines: [...health, ...content.lines].map((line) => (
      line.startsWith(" ") ? line : ` ${truncateToWidth(line, contentWidth)}`
    )),
    detailLineCount: content.detailLineCount,
  };
}

function renderMemoryDetail(
  theme: Theme,
  contentWidth: number,
  detail: MemoryEntry,
  scroll: number,
): MemoryRenderResult {
  const detailLines = wrapTextWithAnsi(detail.content, contentWidth);
  const start = Math.min(scroll, Math.max(0, detailLines.length - MAX_VISIBLE_DETAIL_LINES));
  const end = Math.min(detailLines.length, start + MAX_VISIBLE_DETAIL_LINES);
  const lines = [
    "",
    theme.fg("accent", theme.bold("Raw memory detail")),
    `id ${detail.id}`,
    `scope ${scopeLabel(detail)}`,
    `ordinal ${detail.ordinal}`,
    `created ${detail.created_at}`,
    `agent ${detail.provenance.agent}`,
    `session ${detail.provenance.session_id}`,
    `model ${detail.provenance.model_id ?? "unknown"}`,
    `config ${detail.provenance.config_hash}`,
    "",
  ];
  if (start > 0) lines.push(theme.fg("dim", `↑ ${start} detail lines above`));
  lines.push(...detailLines.slice(start, end));
  if (end < detailLines.length) {
    lines.push(theme.fg("dim", `↓ ${detailLines.length - end} detail lines below`));
  }
  return { lines, detailLineCount: detailLines.length };
}

function renderMemoryNodes(
  theme: Theme,
  contentWidth: number,
  state: MemoryRenderState,
): MemoryRenderResult {
  if (state.nodes.length === 0) {
    return {
      lines: ["", theme.fg("muted", "No memory nodes in this view.")],
      detailLineCount: 0,
    };
  }
  const heading = memoryNodeHeading(state.mode);
  const start = windowStart(state.nodes.length, state.selectedIndex, MAX_VISIBLE_NODES);
  const end = Math.min(state.nodes.length, start + MAX_VISIBLE_NODES);
  const lines = ["", theme.fg("accent", theme.bold(heading))];
  if (state.mode === "root") {
    lines.push(...memoryOmissionLines(theme, state.view));
  }
  if (start > 0) lines.push(theme.fg("dim", `↑ ${start} older nodes above`));
  lines.push(...visibleMemoryNodeLines(theme, contentWidth, state, start, end));
  if (end < state.nodes.length) {
    lines.push(theme.fg("dim", `↓ ${state.nodes.length - end} newer nodes below`));
  }
  return { lines, detailLineCount: 0 };
}

function memoryNodeHeading(mode: MemoryMode): string {
  if (mode === "root") return "Layered memory";
  if (mode === "expanded") return "Expanded summary";
  return "Search results";
}

function visibleMemoryNodeLines(
  theme: Theme,
  contentWidth: number,
  state: MemoryRenderState,
  start: number,
  end: number,
): string[] {
  const lines = [];
  for (let index = start; index < end; index += 1) {
    const node = state.nodes[index];
    if (node === undefined) continue;
    const selected = index === state.selectedIndex;
    const text = truncateToWidth(`${selected ? "› " : "  "}${nodeLine(node)}`, contentWidth);
    lines.push(selected ? theme.fg("accent", text) : text);
  }
  return lines;
}

function memoryOmissionLines(theme: Theme, view: MemoryView | undefined): string[] {
  const omissions = view?.context.omitted ?? [];
  const lines = omissions.slice(0, MAX_VISIBLE_OMISSIONS).map((omission) => {
    const label = omission.scope.scope === "global" ? "global" : "project";
    return theme.fg(
      "warning",
      `omitted ${label} #${omission.start_ordinal}-${omission.end_ordinal} · ${omission.reason}`,
    );
  });
  const hidden = omissions.length - lines.length;
  if (hidden > 0) lines.push(theme.fg("dim", `${hidden} more omitted ranges`));
  return lines;
}

function memoryHealthLines(theme: Theme, view: MemoryView | undefined, status: string): string[] {
  if (view === undefined) return [theme.fg("muted", status)];
  const health = view.health;
  const rawCount = health.scopes.reduce((total, value) => total + value.raw_entries, 0);
  const summaryCount = health.scopes.reduce((total, value) => total + value.summaries, 0);
  const scopes = health.scopes.map((value) => {
    const label = value.scope.scope === "global" ? "global" : "project";
    return `${label} · ${value.raw_entries} raw · ${formatSummaryCount(value.summaries)}`;
  });
  const providerState = health.providers.length === 0
    ? "providers not yet attempted"
    : health.degraded
    ? "providers degraded"
    : "providers healthy";
  const activation = health.last_activation_at === null
    ? "never activated"
    : `activated ${new Date(health.last_activation_at).toLocaleString()}`;
  return [
    `${rawCount} raw · ${formatSummaryCount(summaryCount)} · ${health.pending_summaries} pending`,
    ...scopes,
    `${providerState} · failures ${health.failed_attempts} · cooling ${health.cooling_down_attempts} · fallbacks ${health.fallback_uses}`,
    ...health.providers.map(providerHealthLine),
    `${activation} · ${view.context.item_count} shown · ${view.context.byte_count} bytes`,
    theme.fg("dim", status),
  ];
}

function providerHealthLine(provider: MemoryView["health"]["providers"][number]): string {
  const availability = provider.cooling_down
    ? `cooling until ${provider.next_retry_at === null
      ? "retry window"
      : new Date(provider.next_retry_at).toLocaleString()}`
    : "ready";
  return `${provider.provider} · ${provider.model_id ?? "default model"} · ${provider.last_outcome} · ${availability} · failures ${provider.consecutive_failures} · attempted ${new Date(provider.last_attempted_at).toLocaleString()}`;
}

function initialMemoryPaneState(): MutableMemoryPaneState {
  return {
    active: false,
    generation: 0,
    mode: "root",
    view: undefined,
    nodes: [],
    selectedIndex: 0,
    detail: undefined,
    detailScroll: 0,
    detailLineCount: 0,
    status: "not loaded",
    actionSequence: 0,
    activeAction: undefined,
  };
}

function selectedMemory(state: MutableMemoryPaneState): MemoryNode | undefined {
  return state.nodes[state.selectedIndex];
}

function moveMemorySelection(
  state: MutableMemoryPaneState,
  direction: -1 | 1,
  changed: () => void,
): void {
  if (state.detail !== undefined) {
    const maximum = Math.max(0, state.detailLineCount - MAX_VISIBLE_DETAIL_LINES);
    state.detailScroll = Math.max(0, Math.min(state.detailScroll + direction, maximum));
    changed();
    return;
  }
  if (state.nodes.length === 0) return;
  state.selectedIndex = (state.selectedIndex + direction + state.nodes.length) % state.nodes.length;
  changed();
}

function applyMemoryView(state: MutableMemoryPaneState, view: MemoryView): void {
  state.view = view;
  state.nodes = view.context.nodes;
  state.selectedIndex = 0;
  state.mode = "root";
  state.detail = undefined;
  state.detailScroll = 0;
}

function beginMemoryAction(
  state: MutableMemoryPaneState,
  progress: string,
  changed: () => void,
): RunningMemoryAction | undefined {
  if (!state.active || state.activeAction !== undefined) return undefined;
  const running = {
    generation: state.generation,
    action: ++state.actionSequence,
  };
  state.activeAction = running.action;
  state.status = progress;
  changed();
  return running;
}

function isCurrentMemoryAction(
  state: MutableMemoryPaneState,
  running: RunningMemoryAction,
): boolean {
  return state.active &&
    state.generation === running.generation &&
    state.activeAction === running.action;
}

async function runMemoryAction<T>(
  state: MutableMemoryPaneState,
  dependencies: MemoryPaneDependencies,
  progress: string,
  operation: (current: CurrentAction) => Promise<T | undefined>,
  apply: (result: T) => void,
): Promise<void> {
  const running = beginMemoryAction(state, progress, dependencies.changed);
  if (running === undefined) return;
  const current = () => isCurrentMemoryAction(state, running);
  try {
    const result = await operation(current);
    if (!current()) return;
    if (result !== undefined) apply(result);
    state.status = "live";
  } catch (error) {
    if (current()) state.status = `unavailable · ${errorMessage(error)}`;
  } finally {
    if (state.activeAction === running.action) state.activeAction = undefined;
  }
  if (state.active && state.generation === running.generation) dependencies.changed();
}

async function reloadMemory(
  state: MutableMemoryPaneState,
  dependencies: MemoryPaneDependencies,
): Promise<void> {
  const generation = ++state.generation;
  state.status = "loading";
  dependencies.changed();
  try {
    const view = await dependencies.api.load();
    if (!state.active || generation !== state.generation) return;
    applyMemoryView(state, view);
    state.status = "live";
  } catch (error) {
    if (!state.active || generation !== state.generation) return;
    state.status = `unavailable · ${errorMessage(error)}`;
  }
  dependencies.changed();
}

async function openSelectedMemory(
  state: MutableMemoryPaneState,
  dependencies: MemoryPaneDependencies,
): Promise<void> {
  if (state.activeAction !== undefined) return;
  const selected = selectedMemory(state);
  if (selected === undefined) return;
  if (selected.kind === "raw") {
    state.detail = selected;
    state.detailScroll = 0;
    state.mode = "detail";
    dependencies.changed();
    return;
  }
  await runMemoryAction(
    state,
    dependencies,
    "expanding summary",
    async () => dependencies.api.expand(selected.id),
    (nodes) => {
      state.nodes = nodes;
      state.selectedIndex = 0;
      state.mode = "expanded";
      state.detail = undefined;
      state.detailScroll = 0;
    },
  );
}

async function searchMemory(
  state: MutableMemoryPaneState,
  dependencies: MemoryPaneDependencies,
): Promise<void> {
  await runMemoryAction(state, dependencies, "searching", async (current) => {
    const regex = await dependencies.prompts.searchRegex();
    if (regex === undefined || regex.length === 0 || !current()) return;
    return dependencies.api.search(regex);
  }, (nodes) => {
    state.nodes = nodes;
    state.selectedIndex = 0;
    state.mode = "search";
    state.detail = undefined;
    state.detailScroll = 0;
  });
}

async function addMemory(
  state: MutableMemoryPaneState,
  dependencies: MemoryPaneDependencies,
): Promise<void> {
  await runMemoryAction(state, dependencies, "adding memory", async (current) => {
    const memory = await dependencies.prompts.newMemory();
    if (memory === undefined || memory.content.trim().length === 0 || !current()) return;
    await dependencies.api.add(memory.scope, memory.content);
    if (!current()) return;
    return dependencies.api.load();
  }, (view) => applyMemoryView(state, view));
}

async function invalidateSelectedMemory(
  state: MutableMemoryPaneState,
  dependencies: MemoryPaneDependencies,
): Promise<void> {
  const selected = selectedMemory(state);
  if (selected === undefined || selected.kind !== "summary") return;
  await runMemoryAction(state, dependencies, "invalidating summary", async (current) => {
    if (!await dependencies.prompts.confirmInvalidation(selected) || !current()) return;
    await dependencies.api.invalidate(selected.id);
    if (!current()) return;
    return dependencies.api.load();
  }, (view) => applyMemoryView(state, view));
}

async function consolidateMemory(
  state: MutableMemoryPaneState,
  dependencies: MemoryPaneDependencies,
): Promise<void> {
  await runMemoryAction(state, dependencies, "consolidating", async (current) => {
    await dependencies.api.consolidate();
    if (!current()) return;
    return dependencies.api.load();
  }, (view) => applyMemoryView(state, view));
}

function backFromMemoryView(
  state: MutableMemoryPaneState,
  dependencies: MemoryPaneDependencies,
): void {
  if (state.mode === "root" && state.detail === undefined) return;
  if (state.detail !== undefined && state.mode === "detail") {
    state.detail = undefined;
    state.detailScroll = 0;
    state.mode = "root";
  }
  void reloadMemory(state, dependencies);
}

export class MemoryPane implements Component {
  private readonly state = initialMemoryPaneState();
  private readonly dependencies: MemoryPaneDependencies;

  constructor(
    tui: TUI,
    private readonly theme: Theme,
    api: MemoryApi,
    prompts: MemoryInteractionPrompts,
    updated: () => void = () => undefined,
  ) {
    this.dependencies = {
      api,
      prompts,
      changed() {
        updated();
        tui.requestRender();
      },
    };
  }

  render(width: number): string[] {
    const rendered = renderMemoryPane(this.theme, width, this.state);
    this.state.detailLineCount = rendered.detailLineCount;
    return rendered.lines;
  }

  invalidate(): void {}

  activate(): void {
    if (this.state.active) return;
    this.state.active = true;
    void reloadMemory(this.state, this.dependencies);
  }

  deactivate(): void {
    this.state.active = false;
    this.state.generation += 1;
    this.state.activeAction = undefined;
  }

  dispose(): void {
    this.deactivate();
  }

  refresh(): void {
    if (this.state.active) void reloadMemory(this.state, this.dependencies);
  }

  statusLine(): string {
    const scope = this.state.view?.context.scope.scope ?? "layered";
    return `${scope} memory · ${this.state.status} · MCP local only`;
  }

  handleInput(data: string): void {
    if (matchesKey(data, Key.up) || matchesKey(data, "k")) {
      moveMemorySelection(this.state, -1, this.dependencies.changed);
    } else if (matchesKey(data, Key.down) || matchesKey(data, "j")) {
      moveMemorySelection(this.state, 1, this.dependencies.changed);
    } else if (matchesKey(data, Key.enter)) {
      void openSelectedMemory(this.state, this.dependencies);
    } else if (matchesKey(data, Key.slash)) {
      void searchMemory(this.state, this.dependencies);
    } else if (matchesKey(data, "a")) {
      void addMemory(this.state, this.dependencies);
    } else if (matchesKey(data, "f")) {
      void invalidateSelectedMemory(this.state, this.dependencies);
    } else if (matchesKey(data, "c")) {
      void consolidateMemory(this.state, this.dependencies);
    } else if (matchesKey(data, "r")) {
      this.refresh();
    } else if (
      matchesKey(data, Key.escape) ||
      matchesKey(data, Key.backspace) ||
      matchesKey(data, Key.left)
    ) {
      backFromMemoryView(this.state, this.dependencies);
    }
  }
}
