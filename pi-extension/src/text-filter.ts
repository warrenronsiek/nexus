// @feature observability-ui
// @spec docs/features/observability-ui.md
import { Key, matchesKey } from "@earendil-works/pi-tui";

interface FilterState<Field extends string> {
  editor?: { field: Field; text: string };
  query: Partial<Record<Field, string>>;
}

/** Edit one bounded plain-text filter, returning its field only when the query changes. */
export function editTextFilter<Field extends string>(state: FilterState<Field>, data: string): Field | undefined {
  const editor = state.editor!;
  if (matchesKey(data, Key.enter)) {
    const value = editor.text.trim();
    if (value) state.query[editor.field] = value; else delete state.query[editor.field];
    state.editor = undefined;
    return editor.field;
  }
  if (matchesKey(data, Key.escape)) state.editor = undefined;
  else if (matchesKey(data, Key.backspace)) editor.text = [...editor.text].slice(0, -1).join("");
  else if (!/[\u0000-\u001f\u007f-\u009f]/u.test(data)) editor.text = (editor.text + data).slice(0, 256);
  return undefined;
}
