// @feature code-concerns
// @feature observability-ui
// @spec docs/features/code-concerns.md
// @spec docs/features/observability-ui.md
// @boundary concern-evidence: inspect only recorded structured Git locators without fetching bytes or inferring code identity.
import type { ConcernObservation, ConcernSessionTitle } from "./concerns.ts";
import type { JsonValue } from "./wire-values.ts";

function object(value: JsonValue | undefined): Record<string, JsonValue> | undefined { return typeof value === "object" && value !== null && !Array.isArray(value) ? value : undefined; }
function array(value: JsonValue | undefined): JsonValue[] { return Array.isArray(value) ? value : []; }
function value(item: Record<string, JsonValue>, key: string): string { const found = item[key]; return found === undefined || found === null ? "unknown" : typeof found === "string" ? found : JSON.stringify(found); }
interface LocatedEvidence { location: Record<string, JsonValue>; role: string }
function attributed(candidate: JsonValue | undefined, role: string): LocatedEvidence[] { const location = object(candidate); return location ? [{ location, role }] : []; }
function locations(item: ConcernObservation): LocatedEvidence[] {
  const body = item.input.body, encounter = object(body.encounter);
  const evidence = [...array(body.evidence), ...array(encounter?.evidence)].map(object).filter(item => item?.kind === "git");
  const affected = array(encounter?.areas).map(object).filter((item): item is Record<string, JsonValue> => item !== undefined);
  return [...attributed(body.fixing_location, "fixing_location"), ...evidence.flatMap(item => attributed(item!.location, "evidence")),
    ...affected.flatMap(item => attributed(item.location, value(item, "relation")))];
}
export function concernLocationLines(item: ConcernObservation): string[] {
  const reported = locations(item);
  return [...reported.slice(0, 2).flatMap(({ location, role }) => [
    `Git: ${value(location, "repository")} @ ${value(location, "commit")} (${value(location, "object_format")}) · ${value(location, "path")}`,
    `Location: role ${role} · dirty ${value(location, "dirty")} · verification ${value(location, "verification")} · ${value(location, "qualified_name")}`,
  ]), ...(reported.length > 2 ? [`${reported.length - 2} additional Git locations · CLI history retains every locator`] : [])];
}
export function concernSessionLine(item: ConcernObservation, titles: ConcernSessionTitle[] = []): string {
  const source = object(item.source), original = object(source?.provenance), context = item.input.context;
  const agent = typeof original?.agent === "string" ? original.agent : item.provenance.agent;
  const title = titles.find(title => title.agent === agent && title.host_session_id === context.host_session_id && title.work_session_id === context.work_session_id);
  return `Session names (current): host ${title?.host_title ?? "unavailable"} · work ${title?.work_title ?? "unavailable"}`;
}
