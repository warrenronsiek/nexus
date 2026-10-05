// @feature agent-memory
// @spec docs/features/agent-memory.md
import type { ExtensionUIContext } from "@earendil-works/pi-coding-agent";
import { describe, expect, it, vi } from "vitest";
import { MemoryPrompts } from "../src/memory-prompts.ts";

function ui(overrides: Partial<ExtensionUIContext> = {}): ExtensionUIContext {
  return {
    select: vi.fn(async () => "Project"),
    input: vi.fn(async () => "Keep raw notes immutable."),
    confirm: vi.fn(async () => true),
    ...overrides,
  } as unknown as ExtensionUIContext;
}

describe("Pi memory dialogs", () => {
  it("asks for an explicit project or global scope before adding", async () => {
    const projectUi = ui();
    const globalUi = ui({ select: vi.fn(async () => "Global") });

    await expect(new MemoryPrompts(projectUi).newMemory()).resolves.toEqual({
      scope: "project",
      content: "Keep raw notes immutable.",
    });
    await expect(new MemoryPrompts(globalUi).newMemory()).resolves.toEqual({
      scope: "global",
      content: "Keep raw notes immutable.",
    });
    expect(projectUi.select).toHaveBeenCalledWith("Memory scope", ["Project", "Global"]);
  });

  it("cancels add without opening text input when no scope is selected", async () => {
    const context = ui({ select: vi.fn(async () => undefined) });

    await expect(new MemoryPrompts(context).newMemory()).resolves.toBeUndefined();
    expect(context.input).not.toHaveBeenCalled();
  });

  it("collects regex search and explains that forgetting preserves raw notes", async () => {
    const context = ui({ input: vi.fn(async () => "typed.*boundary") });
    const prompts = new MemoryPrompts(context);
    const summary = {
      kind: "summary" as const,
      id: "summary-1",
      scope: { scope: "global" as const },
      level: 1,
      start_ordinal: 0,
      end_ordinal: 2,
      content: "Use typed boundaries.",
      source_hash: "source-hash",
      provider: "codex",
      model_id: null,
      created_at: "2026-10-04T12:00:00Z",
    };

    await expect(prompts.searchRegex()).resolves.toBe("typed.*boundary");
    await expect(prompts.confirmInvalidation(summary)).resolves.toBe(true);
    expect(context.confirm).toHaveBeenCalledWith(
      "Invalidate derived summary?",
      expect.stringContaining("Raw notes are immutable"),
    );
  });
});
