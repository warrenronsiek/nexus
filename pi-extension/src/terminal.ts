// @feature observability-ui
// @feature runtime
// @feature agent-memory
// @spec docs/features/observability-ui.md
// @spec docs/features/runtime.md
// @spec docs/features/agent-memory.md
// @entrypoint runTerminalDashboard
import { spawn } from "node:child_process";
import { parseArgs } from "node:util";
import { matchesKey, ProcessTerminal, TuiAltScreen } from "@earendil-works/pi-tui";
import { fetchSnapshot, fetchUsage } from "./client.ts";
import { NexusDashboardComponent } from "./component.ts";
import type { Snapshot } from "./domain.ts";
import { NexusMcpClient } from "./mcp-client.ts";
import { MemoryPrompts } from "./memory-prompts.ts";
import { NexusMemoryApi } from "./memory.ts";
import { TerminalDialogs } from "./terminal-dialogs.ts";
import { terminalTheme } from "./theme.ts";

async function runTerminalDashboard(): Promise<void> {
  const { values } = parseArgs({ options: {
    nexus: { type: "string" }, config: { type: "string" },
  } });
  if (!values.nexus) throw new Error("Missing Nexus terminal launch context");
  const cwd = process.cwd();
  const executable = values.nexus;
  const argumentsValue = values.config === undefined ? [] : ["--config", values.config];
  const client = new NexusMcpClient((directory) => spawn(executable, [...argumentsValue, "mcp"], {
    cwd: directory, stdio: ["pipe", "pipe", "ignore"],
  }));
  try {
    const snapshot = await fetchSnapshot(client, cwd, null);
    await showTerminalDashboard(snapshot, client, cwd);
  } finally {
    client.close();
  }
}

async function showTerminalDashboard(snapshot: Snapshot, client: NexusMcpClient, cwd: string): Promise<void> {
  const terminal = new ProcessTerminal();
  const tui = new TuiAltScreen(terminal);
  const dialogs = new TerminalDialogs(tui, terminalTheme);
  const memoryApi = new NexusMemoryApi(client, cwd, () => ({
    session_id: `nexus-tui:${process.pid}`, project_root: cwd, agent: "nexus-tui",
  }));
  let done!: () => void;
  const closed = new Promise<void>((resolve) => { done = resolve; });
  const dashboard = new NexusDashboardComponent(tui, terminalTheme, snapshot, {
    loadSnapshot: (projectId, signal) => fetchSnapshot(client, cwd, projectId, signal),
    loadUsage: (projectId, signal) => fetchUsage(client, cwd, projectId, signal),
    memoryApi, memoryPrompts: new MemoryPrompts(dialogs), done,
  });
  tui.addChild(dashboard);
  tui.setFocus(dashboard);
  tui.addInputListener((data) => matchesKey(data, "ctrl+c") ? (done(), { consume: true }) : undefined);
  process.once("SIGINT", done);
  process.once("SIGTERM", done);
  try {
    tui.start();
    await closed;
  } finally {
    dashboard.dispose();
    dialogs.dispose();
    await terminal.drainInput(100, 20);
    tui.stop();
    process.removeListener("SIGINT", done);
    process.removeListener("SIGTERM", done);
  }
}

runTerminalDashboard().catch((error: unknown) => {
  console.error(`Nexus is unavailable: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
