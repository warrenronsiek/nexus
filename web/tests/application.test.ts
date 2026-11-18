// @vitest-environment jsdom
// @feature audit-registry
// @spec docs/features/audit-registry.md
// @feature observability-ui
// @feature delivery-coordination
// @spec docs/features/delivery-coordination.md
// @spec docs/features/observability-ui.md
// @boundary browser-test: exercise the compiled Elm program against the closed HTTP read contract.
import { readFileSync } from "node:fs";
import { URL as NodeUrl } from "node:url";
import { expect, it, vi } from "vitest";
import { Elm } from "virtual:nexus-elm";
import { renderChart } from "../src/charts";

interface RequestBody { project_id?: string; username?: string; password?: string; query?: { kind: string; work_session_id?: string; group_id?: string; scope?: string; session_id?: string; subject?: {kind: string; id: string}; filter?: { kind?: string; bucket?: string; group_by?: string; measure?: { kind: string } } } }
const requests: { url: string; body: RequestBody; timeout: number }[] = [];
const completions: string[] = [];
const failedReadKinds = new Set<string>();
let configuredInterval = 30000;
let remoteViewer: { default_project_id: string; global_project_reads: false } | null = null;
function fixture(name: string): string {
  return readFileSync(new NodeUrl(`./fixtures/${name}.json`, import.meta.url), "utf8");
}
function response(url: string, body: RequestBody): string {
  if (url.endsWith("/session")) return '{"authenticated":false}';
  if (url.endsWith("/login")) return JSON.stringify({ authenticated: body.username === "observer" && body.password === "fixture-password", username: "observer", ...remoteViewer });
  if (url.endsWith("/logout")) return '{"authenticated":false}';
  return readFixture(body);
}
type ReadQuery = NonNullable<RequestBody["query"]>;
const deliveryDetails: Readonly<Record<string, string>> = { contribution: "delivery_contribution" };
const deliveryLists: Readonly<Record<string, string>> = { rollout: "delivery_rollouts", remediation: "delivery_remediations" };
const fixtureReaders: Readonly<Record<string, (query: ReadQuery) => string>> = {
  coordination: coordinationFixture,
  code_map: query => codeMapFixture(query.filter),
  delivery_get: query => fixture(deliveryDetails[query.subject?.kind ?? ""] ?? query.kind),
  delivery_list: query => fixture(deliveryLists[query.filter?.kind ?? ""] ?? query.kind),
  work_dashboard: query => fixture(query.work_session_id ? "work-session" : query.kind),
  eval_dashboard: query => fixture(query.group_id ? "eval-group" : query.kind),
};
function readFixture(body: RequestBody): string {
  if (!body.query) throw new Error("Browser read omitted its closed query");
  const reader = fixtureReaders[body.query.kind];
  return reader ? reader(body.query) : fixture(body.query.kind);
}
function coordinationFixture(): string {
  const focusedDelivery = window.location.pathname.endsWith("/pull-requests") && window.location.search.includes("record=");
  return JSON.stringify({ ...JSON.parse(fixture(focusedDelivery ? "delivery_coordination" : "coordination")), refresh_interval_ms: configuredInterval });
}
function codeMapFixture(filter: NonNullable<RequestBody["query"]>["filter"]): string {
  const body = JSON.parse(fixture(filter?.bucket ? "code_map_drill" : "code_map"));
  body.map.group_by = filter?.group_by ?? "file";
  if (filter?.measure?.kind === "published") {
    body.map.measure = filter.measure;
    body.map.title = "Branch load";
  }
  return JSON.stringify(body);
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
    try {
      this.status = body.query && failedReadKinds.has(body.query.kind) ? 503 : 200;
      this.response = this.status === 503 ? '{"error":"Fixture read unavailable"}' : response(this.url, body);
    }
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
  document.querySelector<HTMLAnchorElement>(`nav a[href^="/ui/${screen}"]`)!.click();
  await content(expected);
}
function row(value: string): void {
  const element = [...document.querySelectorAll<HTMLButtonElement>(".record-row, .bar-row")].find(button => button.textContent?.includes(value));
  expect(element).toBeDefined(); element!.click();
}
function pick(caption: string, value: string): void {
  const picker = document.querySelector<HTMLSelectElement>(`select[aria-label='${caption}']`)!;
  expect(picker).not.toBeNull();
  picker.value = value; picker.dispatchEvent(new Event("input", { bubbles: true }));
}
async function chartClick(chart: string, label: string): Promise<void> {
  await vi.waitFor(() => expect([...document.querySelectorAll(`#${chart} [role=button]`)].some(group => group.textContent?.includes(label))).toBe(true));
  [...document.querySelectorAll<SVGGElement>(`#${chart} [role=button]`)].find(group => group.textContent?.includes(label))!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
}

async function loginProgram(expectedProject = "fixture-project"): Promise<void> {
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
  await vi.waitFor(() => expect(document.querySelector<HTMLSelectElement>("select[aria-label='Project scope']")?.value).toBe(expectedProject));
  if (!remoteViewer) expect(document.querySelector<HTMLSelectElement>("select[aria-label='Project scope']")!.options[0].value).toBe("");
}
async function inspectCoordination(): Promise<void> {
  await content("Choose a session");
  expect(document.body.textContent).not.toContain("Original response");
  expect(document.body.textContent).not.toContain("Partial history");
  expect(document.body.textContent).not.toContain("Project discovery is bounded");
  await content("Messages · 7 days");
  pick("Session", "host-id");
  await content("Overlap detected with Cache cleanup");
  await content("Nexus → Cache cleanup");
  await content("Overlap with Cache cleanup ended");
  expect(window.location.search).toContain("session=host-id");
  expect(requests.some(request => request.body.query?.kind === "coordination_session_timeline" && request.body.query.session_id === "host-id")).toBe(true);
}
async function inspectWork(): Promise<void> {
  await navigate("work", "Elapsed time by kind of work");
  expect(document.body.textContent).not.toContain("Search and filters");
  pick("Duration aggregation", "total");
  await vi.waitFor(() => expect(document.querySelector("#summary-bars svg")).not.toBeNull());
  pick("Session", "work-id");
  await vi.waitFor(() => expect(document.querySelectorAll("#work-waterfall .waterfall-node").length).toBe(2));
  await chartClick("work-waterfall", "Router tests");
  await content("Router tests · test.unit · 30s · succeeded");
  window.history.back();
  await vi.waitFor(() => expect(document.querySelector("#work-waterfall")).toBeNull());
  await content("Elapsed time by kind of work");
}
async function inspectProjectScope(): Promise<void> {
  pick("Project scope", "");
  await vi.waitFor(() => expect(requests.at(-1)?.body.project_id).toBeNull());
  pick("Project scope", "fixture-project");
  await vi.waitFor(() => expect(requests.at(-1)?.body.project_id).toBe("fixture-project"));
  await vi.waitFor(() => expect(document.querySelector("nav a[href^='/ui/work']")!.getAttribute("href")).toBe("/ui/work?project=fixture-project"));
}
async function inspectEvals(): Promise<void> {
  await navigate("evals", "Choose an agent");
  pick("Agent", "router-eval");
  await content("9007199254740993.0000000001");
}
async function inspectToolsAndSkills(): Promise<void> {
  await navigate("tools", "Code research");
  row("Code research");
  await content("4 runs");
  await vi.waitFor(() => expect(document.querySelectorAll("#commands-0-0 rect").length).toBe(2));
  await navigate("skills", "nexus-work-history");
  row("nexus-work-history");
  await content("Record nested work so sessions can be explained.");
}
async function inspectReports(): Promise<void> {
  await navigate("concerns", "Source occurrences");
  await chartClick("summary-bars", "Router cache");
  await content("Router cache race");
  row("Router cache race"); await content("Seen in the router suite");
  document.querySelector<HTMLButtonElement>(".pagination button")!.click();
  await vi.waitFor(() => expect(requests.at(-1)?.body.query).toEqual({ kind: "concern_list", filter: { area_id: "router-area", after: "32" } }));
  await inspectCodeMap();
  await navigate("papercuts", "Slow local fixture");
  row("Slow local fixture"); await content("Hit it again while running the router tests");
  await navigate("sos", "Unexpected live email");
  row("Unexpected live email"); await content("An email arrived with an unexpected attachment type");
  await navigate("memory", "Keep router evidence attributable");
  await content("Level 1 · 2 notes");
  row("Router evidence notes"); await content("Router tests must not share a cache");
}
async function inspectCodeMap(): Promise<void> {
  const tab = [...document.querySelectorAll<HTMLButtonElement>(".subnav button")].find(button => button.textContent === "Code map");
  expect(tab).toBeDefined();
  tab!.click();
  await content("Reported concern encounters");
  pick("Group by", "team");
  await vi.waitFor(() => expect(requests.at(-1)?.body.query).toMatchObject({ kind: "code_map", filter: { group_by: "team" } }));
  await content("Reported concern encounters");
  pick("Measure", JSON.stringify({kind: "published", producer: "ci", measure: "branch_load"}));
  await vi.waitFor(() => expect(requests.at(-1)?.body.query).toMatchObject({
    kind: "code_map", filter: { group_by: "team", measure: { kind: "published", producer: "ci", measure: "branch_load" } }
  }));
  await content("Branch load");
  pick("Measure", JSON.stringify({kind: "encounters"}));
  await content("Reported concern encounters");
  expect([...document.querySelectorAll("select[aria-label='Group by'] option")].some(option => option.textContent === "team")).toBe(true);
  await chartClick("code-map", "Router cache");
  await vi.waitFor(() => expect(requests.at(-1)?.body.query).toMatchObject({ kind: "code_map", filter: { bucket: "value:cache" } }));
  await chartClick("code-map", "src/router.rs");
  await content("Router cache race");
  row("Router cache race");
  await content("Seen in the router suite");
}
function assertReadOnly(): void {
  expect(document.body.querySelectorAll("input[name=password]")).toHaveLength(0);
  expect([...document.querySelectorAll("button")].map(button => button.textContent).join(" ")).not.toMatch(/add note|invalidate|consolidate/i);
  expect(requests.filter(request => request.url.endsWith("/read")).every(request => request.body.query?.kind && request.body.project_id !== undefined)).toBe(true);
  expect(requests.every(request => request.timeout === 20000)).toBe(true);
}

async function inspectNavigation(): Promise<HTMLButtonElement> {
  const links = [...document.querySelectorAll<HTMLAnchorElement>("nav[aria-label='Workspace'] a")];
  expect(links).toHaveLength(11);
  expect(links.every(link => link.querySelector("svg[aria-hidden='true'] path"))).toBe(true);
  expect(links.map(link => link.textContent)).toEqual(["Coordination", "Work history", "Pull requests", "Audits", "Evaluations", "Tool intelligence", "Skills", "Code concerns", "Papercuts", "SOS", "Agent memory"]);
  expect(links.find(link => link.getAttribute("aria-current") === "page")?.textContent).toBe("Coordination");
  const menu = document.querySelector<HTMLButtonElement>("button[aria-controls='workspace-navigation']")!;
  expect(menu).not.toBeNull();
  expect(menu.getAttribute("aria-expanded")).toBe("false");
  menu.click();
  await vi.waitFor(() => expect(menu.getAttribute("aria-expanded")).toBe("true"));
  return menu;
}

async function inspectPullRequestRegistry(): Promise<void> {
  const link = document.querySelector<HTMLAnchorElement>("nav a[href^='/ui/pull-requests']");
  expect(link, "PR discovery is a labeled workspace destination").not.toBeNull();
  link!.click();
  await content("November release");
  await content("Release shepherd");
  expect(requests.some(request => request.body.query?.kind === "delivery_list")).toBe(true);
}
async function inspectPullRequestEvidence(): Promise<void> {
  document.querySelector<HTMLAnchorElement>(".delivery-row")!.click();
  await Promise.all(["Staging integration", "development → staging", "Cache implementer", "Each request needs an isolated cache", "November promotion", "Investigate staging regression", "AvailabilityUnknown"].map(content));
  expect(window.location.search).toContain("record=pull_request%3Arelease-pr");
  expect(document.body.textContent).toContain("b".repeat(40));
}
async function inspectCurrentDeliveryOverlaps(): Promise<void> {
  await content("Current claims and overlaps");
  const panel = document.querySelector<HTMLElement>("[aria-label='Current delivery overlaps']")!;
  expect(panel).not.toBeNull();
  expect(panel.textContent).toContain("src/cache.rs");
  expect(panel.textContent).toContain("Cache refresh");
  expect(panel.textContent).toContain("Retry policy");
  expect(panel.textContent).toContain("critical");
  expect(panel.textContent).toContain("Coordination sample is truncated");
  expect(panel.textContent).toContain("src/isolated.rs");
  expect(panel.textContent).toContain("No overlap in the observed coordination window.");
  expect(panel.textContent).not.toMatch(/src\/unrelated.rs|Unrelated editor/);
  expect(panel.querySelector("a[href*='session=live-peer']")?.getAttribute("href")).toContain("project=fixture-project");
  expect(requests.filter(request => request.body.query?.kind === "coordination").at(-1)?.body.project_id).toBe("fixture-project");
}
async function inspectOverlapRefreshFailure(): Promise<void> {
  failedReadKinds.add("coordination");
  try {
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Refresh")!.click();
    await content("Current overlap refresh failed.");
    const panel = document.querySelector<HTMLElement>("[aria-label='Current delivery overlaps']")!;
    expect(panel.textContent).toContain("Showing the last successful coordination snapshot.");
    expect(panel.textContent).toContain("2026-11-15T16:00:00Z");
    expect(panel.textContent).toContain("critical overlap");
  } finally {
    failedReadKinds.clear();
  }
}
async function inspectOriginalContribution(): Promise<void> {
  document.querySelector<HTMLAnchorElement>(".delivery-related a[href*='contribution']")!.click();
  await Promise.all(["Why this changed", "old 10–12 → new 10–14", "supplied · squash"].map(content));
  expect(document.querySelector(".delivery-detail-grid")?.textContent).not.toContain("Responsibility and observed availability are separate.");
  expect(document.querySelector("a[href^='/ui/work?session=cache-work']")).not.toBeNull();
}
function deliveryTab(caption: string): void {
  const button = [...document.querySelectorAll<HTMLButtonElement>(".subnav button")].find(candidate => candidate.textContent === caption);
  expect(button, `Delivery tab ${caption} is available`).toBeDefined();
  button!.click();
}
async function inspectDeliveryViews(): Promise<void> {
  deliveryTab("Rollouts");
  await content("November promotion");
  expect(window.location.search, "A list tab clears the previous record deep link").not.toContain("record=");
  expect(requests.filter(request => request.body.query?.kind === "delivery_list").at(-1)?.body.query?.filter?.kind).toBe("rollout");
  deliveryTab("Remediation");
  await content("Investigate staging regression");
}
function assertDeliveryReadOnly(): void {
  expect([...document.querySelectorAll("button")].map(button => button.textContent).join(" ")).not.toMatch(/assign|deploy|merge|send|resume/i);
  const reads = requests.filter(request => request.body.query?.kind.startsWith("delivery_"));
  expect(reads.every(request => ["delivery_list", "delivery_get", "delivery_watches"].includes(request.body.query!.kind))).toBe(true);
}

it("discovers PR shepherds and reads rollout evidence without exposing delivery actions", async () => {
  window.history.replaceState(null, "", "/ui/coordination");
  await loginProgram();
  await inspectPullRequestRegistry();
  await inspectPullRequestEvidence();
  await inspectCurrentDeliveryOverlaps();
  await inspectOverlapRefreshFailure();
  await inspectOriginalContribution();
  await inspectDeliveryViews();
  assertDeliveryReadOnly();
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Sign out")!.click();
  await content("Welcome to Nexus");
  vi.unstubAllGlobals();
});

it("runs the actual Elm login, eleven destinations, pickers, inline evidence and exact eval values through read-only HTTP", async () => {
  window.history.replaceState(null, "", "/ui/coordination");
  await loginProgram();
  const menu = await inspectNavigation();
  await inspectCoordination();
  await inspectProjectScope();
  await inspectWork();
  expect(menu.getAttribute("aria-expanded")).toBe("false");
  await inspectEvals();
  await inspectToolsAndSkills();
  await inspectReports();
  assertReadOnly();
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Sign out")!.click();
  await content("Welcome to Nexus");
  expect(requests.at(-1)?.url).toBe("/ui/api/logout");
  expect(requests.at(-1)?.timeout).toBe(20000);
  vi.unstubAllGlobals();
});

function readCount(kind: string): number {
  return requests.filter(request => request.body.query?.kind === kind).length;
}
async function pollingTick(kind: string): Promise<void> {
  const before = readCount(kind);
  await vi.advanceTimersByTimeAsync(configuredInterval);
  await vi.waitFor(() => expect(readCount(kind)).toBe(before + 1));
}

it("honors configured refresh cadence after direct Work login and retains it across navigation", async () => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  const intervals = vi.spyOn(globalThis, "setInterval");
  configuredInterval = 750;
  requests.length = 0; completions.length = 0;
  window.history.replaceState(null, "", "/ui/work");
  try {
    await loginProgram();
    await vi.waitFor(() => expect(intervals).toHaveBeenCalledWith(expect.any(Function), 750));
    await pollingTick("work_dashboard");
    await navigate("evals", "Router / claude + codex");
    await pollingTick("eval_dashboard");
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Sign out")!.click();
    await content("Welcome to Nexus");
    const beforeLogout = requests.length;
    await vi.advanceTimersByTimeAsync(1500);
    expect(requests.length).toBe(beforeLogout);
  } finally {
    configuredInterval = 30000;
    vi.restoreAllMocks(); vi.useRealTimers(); vi.unstubAllGlobals();
  }
});

async function switchRemoteProject(): Promise<void> {
  const scope = document.querySelector<HTMLSelectElement>("select[aria-label='Project scope']")!;
  expect([...scope.options].some(option => option.value === "")).toBe(false);
  expect(requests.filter(request => ["coordination", "work_dashboard"].includes(request.body.query?.kind ?? "")).every(request => request.body.project_id === "remote-project-a")).toBe(true);
  scope.value = "fixture-project"; scope.dispatchEvent(new Event("input", { bubbles: true }));
  await vi.waitFor(() => expect(requests.at(-1)?.body.project_id).toBe("fixture-project"));
  await content("Elapsed time by kind of work");
  await pollingTick("work_dashboard");
  expect(requests.at(-1)?.body.project_id).toBe("fixture-project");
}

it("keeps native remote reads honestly scoped to the configured project and later selection", async () => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  const intervals = vi.spyOn(globalThis, "setInterval");
  configuredInterval = 900;
  remoteViewer = { default_project_id: "remote-project-a", global_project_reads: false };
  requests.length = 0; completions.length = 0;
  window.history.replaceState(null, "", "/ui/work");
  try {
    await loginProgram("remote-project-a");
    await vi.waitFor(() => expect(intervals).toHaveBeenCalledWith(expect.any(Function), 900));
    await switchRemoteProject();
    [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Sign out")!.click();
    await content("Welcome to Nexus");
  } finally {
    remoteViewer = null; configuredInterval = 30000;
    vi.restoreAllMocks(); vi.useRealTimers(); vi.unstubAllGlobals();
  }
});

const auditFilter = { control_key: "review", version: 1, from: "2026-11-01T00:00:00Z", until: "2026-12-01T00:00:00Z" };

async function inspectAuditHistoryNavigation(): Promise<void> {
  const detailUrl = new URL(window.location.href);
  expect(detailUrl.searchParams.get("run")).toBe("review-findings");
  expect(detailUrl.searchParams.get("cutoff")).toBe("12");
  window.history.back();
  await content("2 recorded executions · Coverage unknown");
  expect(requests.at(-1)?.body.query).toMatchObject({ kind: "audit_list", filter: auditFilter });
  window.history.forward();
  await content("Independent reviewer");
  expect(requests.at(-1)?.body.query).toMatchObject({ kind: "audit_get", run_id: "review-findings", cutoff: 12 });
}

it("filters audit executions, drills from a result bar, and reads a verified disposition", async () => {
  window.history.replaceState(null, "", "/ui/audits?project=fixture-project");
  await loginProgram();
  await content("2 recorded executions · Coverage unknown");
  input("audit-control", "review");
  input("audit-version", "1");
  input("audit-from", "2026-11-01");
  input("audit-until", "2026-12-01");
  document.querySelector(".audit-filters")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await vi.waitFor(() => expect(requests.at(-1)?.body.query).toMatchObject({
    kind: "audit_list",
    filter: auditFilter,
  }));
  await chartClick("audit-results", "Changes requested");
  await content("Executions in selected bar");
  expect(document.querySelectorAll(".audit-run")).toHaveLength(1);
  row("review-findings");
  await content("Independent reviewer");
  expect(document.body.textContent).toContain("Inspect the fix");
  expect(document.body.textContent).toContain("Run both review passes");
  expect(requests.at(-1)?.body.query).toMatchObject({ kind: "audit_get", run_id: "review-findings", cutoff: 12 });
  expect(document.body.textContent).toContain("Original review recommendation");
  await inspectAuditHistoryNavigation();
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Sign out")!.click();
  await content("Welcome to Nexus");
  vi.unstubAllGlobals();
});
