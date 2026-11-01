// @feature observability-ui
// @spec docs/features/observability-ui.md
import { Elm } from "virtual:nexus-elm";
import { renderChart } from "./charts";
import "./style.css";

const app = Elm.Main.init({ node: document.getElementById("nexus")!, flags: null });
app.ports.charts.subscribe(charts => {
  requestAnimationFrame(() => charts.forEach(chart => {
    const host = document.getElementById(chart.id);
    if (host) renderChart(host, chart, event => app.ports.chartSelection.send(event));
  }));
});
