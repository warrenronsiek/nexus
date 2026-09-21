// @feature observability-ui
// @spec docs/features/observability-ui.md
// @entrypoint browser-bootstrap
// @boundary elm-port-json
import { renderActivityChart, type ActivityBucket } from "./activity-chart";

interface ElmApp {
  ports: {
    renderActivityChart: {
      subscribe(callback: (buckets: ActivityBucket[]) => void): void;
    };
  };
}

declare const Elm: {
  Main: {
    init(options: { node: HTMLElement }): ElmApp;
  };
};

const node = document.querySelector<HTMLElement>("#app");
if (node === null) {
  throw new Error("Nexus UI root is missing");
}

const app = Elm.Main.init({ node });
let latestBuckets: ActivityBucket[] = [];
app.ports.renderActivityChart.subscribe((buckets) => {
  latestBuckets = buckets;
  renderActivityChart(latestBuckets);
});

const appRoot = document.querySelector<HTMLElement>("#app");
if (appRoot !== null) {
  new ResizeObserver(() => renderActivityChart(latestBuckets)).observe(appRoot);
}
