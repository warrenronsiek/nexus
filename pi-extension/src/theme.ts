// @feature observability-ui
// @spec docs/features/observability-ui.md
import type { Component, SelectListTheme } from "@earendil-works/pi-tui";

export type DashboardColor = "accent" | "muted" | "dim" | "warning" | "success" | "error";

export interface DashboardTheme {
  fg(color: DashboardColor, text: string): string;
  bold(text: string): string;
}

export function selectListTheme(theme: DashboardTheme): SelectListTheme {
  return {
    selectedPrefix: (text) => theme.fg("accent", text),
    selectedText: (text) => theme.fg("accent", text),
    description: (text) => theme.fg("muted", text),
    scrollInfo: (text) => theme.fg("dim", text),
    noMatch: (text) => theme.fg("warning", text),
  };
}

const colors: Record<DashboardColor, number> = {
  accent: 36, muted: 37, dim: 90, warning: 33, success: 32, error: 31,
};

export const terminalTheme: DashboardTheme = {
  fg: (color, text) => process.env.NO_COLOR === undefined
    ? `\x1b[${colors[color]}m${text}\x1b[39m` : text,
  bold: (text) => process.env.NO_COLOR === undefined ? `\x1b[1m${text}\x1b[22m` : text,
};

export class DashboardBorder implements Component {
  constructor(private readonly color: (text: string) => string) {}
  invalidate(): void {}
  render(width: number): string[] {
    return [this.color("─".repeat(Math.max(0, width)))];
  }
}
