import { createSignal } from "solid-js";
import { SessionInfo, call } from "../api";
import { errorText } from "../appError";
import { setError } from "./notify";
import { refreshSessions } from "./sessions";

// ---- tabs of the selected worktree: names, order and the ones just closed ----
/** names the user gave tabs: shown at once, and also sent to the daemon so they survive a restart (the local copy covers a failed call) */
const [tabNames, setTabNames] = createSignal<Record<number, string>>({});
export const tabLabel = (s: SessionInfo) => tabNames()[s.id] ?? s.label ?? `Sessão ${s.id}`;
export const renameTab = (id: number, name: string) => {
  const n = name.trim();
  setTabNames((cur) => {
    const next = { ...cur };
    if (n) next[id] = n;
    else delete next[id];
    return next;
  });
  // the local name already shows; if the daemon could not keep it, say so (it would be lost on restart)
  void call("session.rename", { id, label: n })
    .then(() => refreshSessions())
    .catch((e) => setError(`Não foi possível guardar o nome do separador: ${errorText(e)}`));
};
/** the order the user dragged tabs into; ids not in it follow in their natural order */
const [tabOrder, setTabOrder] = createSignal<number[]>([]);
export const orderTabs = <T extends { id: number }>(list: T[], order: number[]): T[] => {
  const at = (x: T) => {
    const i = order.indexOf(x.id);
    return i < 0 ? Infinity : i;
  };
  return list.map((x, i) => [x, i] as const).sort(([a, i], [b, j]) => (at(a) === at(b) ? i - j : at(a) < at(b) ? -1 : 1)).map(([x]) => x);
};
/** the new order after dragging `id` before `target` (null = to the end), given the ids on screen in their current order */
export const moveBefore = (order: number[], shown: number[], id: number, target: number | null): number[] => {
  if (id === target) return order;
  const rest = shown.filter((x) => x !== id);
  const at = target === null ? rest.length : rest.indexOf(target);
  if (at < 0) return order;
  rest.splice(at, 0, id);
  return [...order.filter((x) => !shown.includes(x)), ...rest];
};
export const moveTab = (shown: number[], id: number, target: number | null) => setTabOrder((cur) => moveBefore(cur, shown, id, target));
export const sortedTabs = <T extends { id: number }>(list: T[]) => orderTabs(list, tabOrder());

/** tabs closed recently, newest last, so Ctrl+Shift+T can bring one back */
export type ClosedTab = { label: string | null; meta: SessionInfo["meta"] };
const CLOSED_MAX = 10;
const [closedTabs, setClosedTabs] = createSignal<ClosedTab[]>([]);
export const rememberClosed = (s: SessionInfo) => setClosedTabs((cur) => [...cur, { label: tabNames()[s.id] ?? s.label, meta: s.meta }].slice(-CLOSED_MAX));
/** the most recently closed tab, taken off the list; null when there is none */
export const popClosed = (): ClosedTab | null => {
  const cur = closedTabs();
  if (!cur.length) return null;
  setClosedTabs(cur.slice(0, -1));
  return cur[cur.length - 1];
};
/** forgets names and positions of sessions that no longer exist */
export const pruneTabState = (list: SessionInfo[]) => {
  const alive = new Set(list.map((s) => s.id));
  if (Object.keys(tabNames()).some((k) => !alive.has(Number(k)))) setTabNames((cur) => Object.fromEntries(Object.entries(cur).filter(([k]) => alive.has(Number(k)))));
  if (tabOrder().some((id) => !alive.has(id))) setTabOrder((cur) => cur.filter((id) => alive.has(id)));
};

