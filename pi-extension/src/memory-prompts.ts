// @feature agent-memory
// @spec docs/features/agent-memory.md
import type { MemoryInteractionPrompts } from "./memory-pane.ts";
import type { MemorySummary } from "./memory.ts";

export interface MemoryPromptUI {
  input(title: string, placeholder: string): Promise<string | undefined>;
  select(title: string, choices: string[]): Promise<string | undefined>;
  confirm(title: string, message: string): Promise<boolean>;
}

export class MemoryPrompts implements MemoryInteractionPrompts {
  constructor(private readonly ui: MemoryPromptUI) {}

  searchRegex(): Promise<string | undefined> {
    return this.ui.input("Search memory (Rust regex)", "typed.*boundary");
  }

  async newMemory(): Promise<{
    scope: "global" | "project";
    content: string;
  } | undefined> {
    const selection = await this.ui.select("Memory scope", ["Project", "Global"]);
    if (selection !== "Project" && selection !== "Global") return;
    const content = await this.ui.input("Add immutable memory", "What should Nexus remember?");
    if (content === undefined || content.trim().length === 0) return;
    return {
      scope: selection === "Project" ? "project" : "global",
      content: content.trim(),
    };
  }

  confirmInvalidation(summary: MemorySummary): Promise<boolean> {
    return this.ui.confirm(
      "Invalidate derived summary?",
      `Forget summary ${summary.id} (#${summary.start_ordinal}-${summary.end_ordinal}) and rebuild it during consolidation? Raw notes are immutable and will remain stored.`,
    );
  }
}
