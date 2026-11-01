// @vitest-environment jsdom
// @feature observability-ui
// @spec docs/features/observability-ui.md
// @boundary browser-test: exercise the compiled Elm program against the closed HTTP read contract.
import { readFileSync } from "node:fs";
import { URL as NodeUrl } from "node:url";
import { expect, it, vi } from "vitest";
import { Elm } from "virtual:nexus-elm";
import { renderChart } from "../src/charts";

interface RequestBody { project_id?: string; username?: string; password?: string; query?: { kind: string; work_session_id?: string; group_id?: string; scope?: string; filter?: Record<string, string> } }
const requests: { url: string; body: RequestBody; timeout: number }[] = [];
const completions: string[] = [];
function fixture(name: string): string {
  return readFileSync(new NodeUrl(`./fixtures/${name}.json`, import.meta.url), "utf8");
}
function response(url: string, body: RequestBody): string {
  if (url.endsWith("/session")) return '{"authenticated":false}';
  if (url.endsWith("/login")) return JSON.stringify({ authenticated: body.username === "observer" && body.password === "fixture-password", username: "observer" });
  if (url.endsWith("/logout")) return '{"authenticated":false}';
  return readFixture(body);
}
function readFixture(body: RequestBody): string {
  if (!body.query) throw new Error("Browser read omitted its closed query");
  const query = body.query;
  if (query.kind === "work_dashboard" && query.work_session_id) return fixture("work-session");
  if (query.kind === "eval_dashboard" && query.group_id) return fixture("eval-group");
  return fixture(query.kind);
}
class FixtureXhr extends EventTarget {
  status = 200;
  statusText = "OK";
  response = "";
  responseURL = "";
  timeout = 0;
  private url = "";
  open(_method: string, url: string): void { this.url = url; this.responseURL = url; }
  setRequestHeader(): void {}
  getAllResponseHeaders(): string { return "content-type: application/json\r\n"; }
  abort(): void {}
  send(raw: string | undefined): void {
    const body = raw ? JSON.parse(raw) as RequestBody : {};
    requests.push({ url: this.url, body, timeout: this.timeout });
    try { this.response = response(this.url, body); }
    catch (error) { throw new Error(`Fixture failed for ${body.query?.kind}: ${String(error)}`); }
    setTimeout(() => { completions.push(`${this.url} ${this.response.length}`); this.dispatchEvent(new Event("load")); }, 0);
  }
}
function input(name: string, value: string): void {
  const element = document.querySelector<HTMLInputElement>(`input[name=${name}]`)!;
  element.value = value;
  element.dispatchEvent(new Event("input", { bubbles: true }));
}
async function content(value: string): Promise<void> {
  await vi.waitFor(() => expect(document.body.textContent, JSON.stringify({ requests: requests.map(request => ({ url: request.url, query: request.body.query })), completions })).toContain(value));
}
async function navigate(screen: string, expected: string): Promise<void> {
  document.querySelector<HTMLAnchorElement>(`a[href="/ui/${screen}"]`)!.click();
  await content(expected);
}
function row(value: string): void {
  const element = [...document.querySelectorAll<HTMLButtonElement>(".record-row")].find(button => button.textContent?.includes(value));
  expect(element).toBeDefined(); element!.click();
}
function filterInput(caption: string, value: string): void {
  const label = [...document.querySelectorAll<HTMLLabelElement>(".filter-panel label")].find(label => label.textContent?.includes(caption))!;
  const input = label.querySelector("input")!;
  input.value = value; input.dispatchEvent(new Event("input", { bubbles: true }));
}

async function loginProgram(): Promise<void> {
  vi.stubGlobal("XMLHttpRequest", FixtureXhr);
  document.body.replaceChildren(Object.assign(document.createElement("div"), { id: "app" }));
  const app = Elm.Main.init({ node: document.getElementById("app")!, flags: null });
  app.ports.charts.subscribe(charts => requestAnimationFrame(() => charts.forEach(chart => {
    const host = document.getElementById(chart.id);
    if (host) renderChart(host, chart, event => app.ports.chartSelection.send(event));
  })));
  await content("Welcome to Nexus");
  input("username", "observer"); input("password", "fixture-password");
  document.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await content("Improve the router");
  expect(document.querySelector<HTMLSelectElement>("select[aria-label='Project scope']")!.value).toBe("fixture-project");
  expect(document.querySelector<HTMLSelectElement>("select[aria-label='Project scope']")!.options[0].value).toBe("");
  await content("Project discovery is bounded");
}
async function inspectWork(): Promise<void> {
  await navigate("work", "Sessions measured");
  const aggregation = document.querySelector<HTMLSelectElement>(".duration-controls select")!;
  aggregation.value = "total"; aggregation.dispatchEvent(new Event("input", { bubbles: true }));
  await content("total elapsed duration");
  await navigate("evals", "Router / claude + codex");
  await navigate("work", "Sessions measured");
  expect(document.querySelector<HTMLSelectElement>(".duration-controls select")!.value).toBe("total");
  row("Improve the router"); await content("Task metadata");
  await vi.waitFor(() => expect(document.querySelectorAll("#work-waterfall rect").length).toBe(2));
  const task = [...document.querySelectorAll<SVGGElement>("#work-waterfall [role=button]")].find(group => group.textContent?.includes("Router tests"))!;
  task.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  await content("Selected task · Router tests");
}
async function inspectProjectCatalog(): Promise<void> {
  input("project_id", "archived-project");
  document.querySelector(".project-catalog form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await vi.waitFor(() => expect(requests.at(-1)?.body.project_id).toBe("archived-project"));
  await vi.waitFor(() => expect(document.querySelector<HTMLSelectElement>("select[aria-label='Project scope']")!.value).toBe("archived-project"));
  const scope = document.querySelector<HTMLSelectElement>("select[aria-label='Project scope']")!;
  scope.value = ""; scope.dispatchEvent(new Event("input", { bubbles: true }));
  await vi.waitFor(() => expect(requests.at(-1)?.body.project_id).toBeNull());
  scope.value = "fixture-project"; scope.dispatchEvent(new Event("input", { bubbles: true }));
  await vi.waitFor(() => expect(requests.at(-1)?.body.project_id).toBe("fixture-project"));
}
async function inspectEvals(): Promise<void> {
  await navigate("evals", "Router / claude + codex");
  row("Router / claude + codex"); await content("9007199254740993.0000000001");
}
async function inspectOtherSurfaces(): Promise<void> {
  await navigate("tools", "Code research");
  await navigate("skills", "nexus-work-history");
  await navigate("concerns", "Source occurrences");
  row("Router cache"); await content("Router cache race");
  filterInput("Search concerns", "not-applied");
  document.querySelector<HTMLButtonElement>(".pagination button")!.click();
  await vi.waitFor(() => expect(requests.at(-1)?.body.query).toEqual({ kind: "concern_list", filter: { area_id: "router-area", after: "32" } }));
  await navigate("papercuts", "Slow local fixture");
  await navigate("sos", "Unexpected live email");
  await navigate("memory", "Keep router evidence attributable");
}
async function inspectMemoryScope(): Promise<void> {
  const label = [...document.querySelectorAll<HTMLLabelElement>(".filter-panel label")].find(label => label.textContent?.includes("Scope:"))!;
  const scope = label.querySelector("input")!;
  scope.value = "global"; scope.dispatchEvent(new Event("input", { bubbles: true }));
  document.querySelector(".filter-panel form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await vi.waitFor(() => expect(requests.at(-1)?.body.query).toEqual({ kind: "memory_context", scope: "global" }));
}
function assertReadOnly(): void {
  expect(document.body.querySelectorAll("input[name=password]")).toHaveLength(0);
  expect([...document.querySelectorAll("button")].map(button => button.textContent).join(" ")).not.toMatch(/add note|invalidate|consolidate/i);
  expect(requests.filter(request => request.url.endsWith("/read")).every(request => request.body.query?.kind && request.body.project_id !== undefined)).toBe(true);
  expect(requests.every(request => request.timeout === 20000)).toBe(true);
}

it("runs the actual Elm login, nine screens, work selection and exact eval values through read-only HTTP", async () => {
  await loginProgram();
  await inspectProjectCatalog();
  await inspectWork();
  await inspectEvals();
  await inspectOtherSurfaces();
  await inspectMemoryScope();
  assertReadOnly();
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Sign out")!.click();
  await content("Welcome to Nexus");
  expect(requests.at(-1)?.url).toBe("/ui/api/logout");
  expect(requests.at(-1)?.timeout).toBe(20000);
  vi.unstubAllGlobals();
});
