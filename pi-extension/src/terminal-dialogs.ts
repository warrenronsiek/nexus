// @feature observability-ui
// @feature agent-memory
// @spec docs/features/observability-ui.md
// @spec docs/features/agent-memory.md
import { Container, Input, Key, matchesKey, SelectList, Text, type TUI } from "@earendil-works/pi-tui";
import type { MemoryPromptUI } from "./memory-prompts.ts";
import { DashboardBorder, selectListTheme, type DashboardTheme } from "./theme.ts";

class PromptDialog extends Container {
  constructor(
    private readonly control: Input | SelectList,
    private readonly tui: TUI,
    private readonly cancel: () => void,
  ) { super(); }

  get focused(): boolean { return this.control instanceof Input && this.control.focused; }
  set focused(value: boolean) {
    if (this.control instanceof Input) this.control.focused = value;
  }

  handleInput(data: string): void {
    if (matchesKey(data, Key.escape)) this.cancel();
    else this.control.handleInput(data);
    this.tui.requestRender();
  }
}

export class TerminalDialogs implements MemoryPromptUI {
  private cancel: (() => void) | undefined;

  constructor(private readonly tui: TUI, private readonly theme: DashboardTheme) {}

  input(title: string, placeholder: string): Promise<string | undefined> {
    return this.prompt(title, new Input({ placeholder }));
  }

  select(title: string, choices: string[]): Promise<string | undefined> {
    const control = new SelectList(choices.map((choice) => ({ value: choice, label: choice })), choices.length, selectListTheme(this.theme));
    return this.prompt(title, control);
  }

  async confirm(title: string, message: string): Promise<boolean> {
    return await this.select(`${title}\n${message}`, ["Cancel", "Invalidate"]) === "Invalidate";
  }

  dispose(): void { this.cancel?.(); }

  private prompt(title: string, control: Input | SelectList): Promise<string | undefined> {
    return new Promise((resolve) => {
      const finish = (value: string | undefined): void => {
        overlay.hide();
        this.cancel = undefined;
        resolve(value);
        this.tui.requestRender();
      };
      this.cancel = () => finish(undefined);
      if (control instanceof Input) control.onSubmit = finish;
      else control.onSelect = (item) => finish(item.value);
      const dialog = new PromptDialog(control, this.tui, this.cancel);
      dialog.addChild(new DashboardBorder((text) => this.theme.fg("accent", text)));
      dialog.addChild(new Text(this.theme.bold(title), 1, 1));
      dialog.addChild(control);
      dialog.addChild(new Text(this.theme.fg("dim", "enter confirm · esc cancel"), 1, 1));
      const overlay = this.tui.showOverlay(dialog, { width: "80%", maxHeight: "80%" });
      this.tui.requestRender();
    });
  }
}
