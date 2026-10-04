// @feature observability-ui
// @spec docs/features/observability-ui.md
// @entrypoint startNexus
// @boundary dynamic-json
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { decodeDashboard, decodeProjects, type Snapshot } from "./domain.ts";

const execFileAsync = promisify(execFile);

export function localNexusUrl(output: string): URL {
  const candidate = output
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter(Boolean)
    .at(-1);
  if (!candidate) throw new Error("nexus ui did not print a URL");

  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    throw new Error(`nexus ui printed an invalid URL: ${candidate}`);
  }
  const loopbackHosts = new Set(["127.0.0.1", "localhost", "[::1]", "::1"]);
  if (url.protocol !== "http:" || !loopbackHosts.has(url.hostname) || url.username || url.password) {
    throw new Error(`refusing non-loopback Nexus URL: ${candidate}`);
  }
  return url;
}

export async function startNexus(cwd: string): Promise<URL> {
  const { stdout } = await execFileAsync("nexus", ["ui", "--launch", "print"], {
    cwd,
    timeout: 5000,
    maxBuffer: 64 * 1024,
  });
  return localNexusUrl(stdout);
}

async function responseJson(response: Response, endpoint: string): Promise<unknown> {
  if (!response.ok) throw new Error(`${endpoint} returned HTTP ${response.status}`);
  return response.json() as Promise<unknown>;
}

export async function fetchSnapshot(
  baseUrl: URL,
  projectId: string | null,
  signal?: AbortSignal,
): Promise<Snapshot> {
  const dashboardUrl = new URL("/api/v1/dashboard", baseUrl);
  if (projectId !== null) dashboardUrl.searchParams.set("project_id", projectId);
  const projectsUrl = new URL("/api/v1/projects", baseUrl);
  const [dashboardResponse, projectsResponse] = await Promise.all([
    fetch(dashboardUrl, { signal }),
    fetch(projectsUrl, { signal }),
  ]);
  const [dashboardJson, projectsJson] = await Promise.all([
    responseJson(dashboardResponse, "Nexus dashboard"),
    responseJson(projectsResponse, "Nexus projects"),
  ]);
  return {
    dashboard: decodeDashboard(dashboardJson),
    projects: decodeProjects(projectsJson),
  };
}
