// @feature observability-ui
// @spec docs/features/observability-ui.md
declare module "virtual:nexus-elm" {
  export const Elm: { Main: { init(options: { node: HTMLElement; flags: null }): { ports: { charts: { subscribe(callback: (charts: import("./charts").ChartInput[]) => void): void }; chartSelection: { send(event: import("./charts").ChartEvent): void } } } } };
}
declare module "*.css";
