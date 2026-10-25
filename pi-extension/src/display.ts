// @feature observability-ui
// @spec docs/features/observability-ui.md
// @boundary untrusted-terminal-text
import { stripTerminalSequences, truncateToWidth } from "@earendil-works/pi-tui";

const TERMINAL_CONTROL = /[\u0000-\u001f\u007f-\u009f]/gu;
const REPLACEMENT_CHARACTER = "\ufffd";

export type DisplayTextMode = "single-line" | "multiline";

/** Fit plain display text without retaining the renderer's inserted ANSI reset codes. */
export function fitDisplayText(value: string, width: number): string {
  return stripTerminalSequences(truncateToWidth(sanitizeDisplayText(value), Math.max(1, width), "…"));
}

export function centeredWindowStart(
  total: number,
  selected: number,
  maximum: number,
): number {
  const centered = selected - Math.floor(maximum / 2);
  return Math.max(0, Math.min(centered, Math.max(0, total - maximum)));
}

/**
 * Neutralize terminal controls at the last boundary before external text is rendered.
 * Multiline presentation keeps logical line breaks while sanitizing each line.
 */
export function sanitizeDisplayText(
  value: string,
  mode: DisplayTextMode = "single-line",
): string {
  if (mode === "multiline") {
    return value
      .split(/\r\n?|\n/u)
      .map((line) => line.replace(TERMINAL_CONTROL, REPLACEMENT_CHARACTER))
      .join("\n");
  }
  return value.replace(TERMINAL_CONTROL, REPLACEMENT_CHARACTER);
}
