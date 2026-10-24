import { createSignal } from "solid-js";
import type { ShipStatus } from "./api";
import { readString, writeJson } from "./persist";
import { L } from "./strings.logic";

export type NotifKind = "done" | "waiting" | "ci-fail" | "ci-pass" | "merged" | "error" | "budget" | "attention";

export type NotifEntry = {
  id: string;
  /** epoch ms */
  at: number;
  kind: NotifKind;
  /** worktree path; empty for an event that belongs to none */
  worktree: string;
  text: string;
  /** agent session to open on click, when the event came from one */
  session?: number;
  read: boolean;
};

export type NewEntry = { kind: NotifKind; worktree: string; text: string; session?: number; at?: number };

export const MAX_ENTRIES = 100;
/** the same event again inside this window is the same event (a poll that fires twice, a retry) */
export const DEDUPE_MS = 2 * 60_000;
const KEY = "axelstra.notifications";
const KINDS: NotifKind[] = ["done", "waiting", "ci-fail", "ci-pass", "merged", "error", "budget", "attention"];

/** newest first, at most `max` */
export const trim = (list: NotifEntry[], max = MAX_ENTRIES): NotifEntry[] => list.slice(0, max);

export const isDuplicate = (list: NotifEntry[], e: NewEntry, now: number): boolean =>
  list.some((x) => x.kind === e.kind && x.worktree === e.worktree && x.text === e.text && now - x.at < DEDUPE_MS);

/** the list with `e` on top, or the same list when it repeats a recent entry */
export const addEntry = (list: NotifEntry[], e: NewEntry, now: number, id: string): NotifEntry[] => {
  const at = e.at ?? now;
  if (isDuplicate(list, e, at)) return list;
  return trim([{ id, at, kind: e.kind, worktree: e.worktree, text: e.text, session: e.session, read: false }, ...list]);
};

/** what localStorage held, tolerating anything: garbage gives an empty list, bad entries are dropped */
export const parseStored = (raw: string | null): NotifEntry[] => {
  if (!raw) return [];
  try {
    const v = JSON.parse(raw);
    if (!Array.isArray(v)) return [];
    const ok = v.filter(
      (x): x is NotifEntry =>
        !!x && typeof x.id === "string" && typeof x.at === "number" && KINDS.includes(x.kind) && typeof x.worktree === "string" && typeof x.text === "string",
    )
      // early builds logged every agent that merely went quiet as an error: those are noise, drop them on load
      .filter((x) => !(x.kind === "error" && x.text.includes("ficou parado")));
    return trim(
      ok
        .map((x) => ({ id: x.id, at: x.at, kind: x.kind, worktree: x.worktree, text: x.text, session: typeof x.session === "number" ? x.session : undefined, read: x.read === true }))
        .sort((a, b) => b.at - a.at),
    );
  } catch {
    return [];
  }
};

export const unreadOf = (list: NotifEntry[]) => list.filter((e) => !e.read).length;

const load = () => parseStored(readString(KEY));
const save = (list: NotifEntry[]) => writeJson(KEY, list); // private mode or full: the list lives on in memory

export const [notifications, setNotifications] = createSignal<NotifEntry[]>(load());
export const unreadCount = () => unreadOf(notifications());

let seq = 0;
const update = (f: (l: NotifEntry[]) => NotifEntry[]) => {
  const next = f(notifications());
  if (next === notifications()) return;
  setNotifications(next);
  save(next);
};

/** records an event in the notification center */
export const push = (e: NewEntry) => update((l) => addEntry(l, e, Date.now(), `${Date.now()}-${seq++}`));
/** the one call the rest of the app makes, e.g. next to notify_session */
export const pushNotification = push;
export const markAllRead = () => update((l) => (l.some((e) => !e.read) ? l.map((e) => (e.read ? e : { ...e, read: true })) : l));
export const clearAll = () => update((l) => (l.length ? [] : l));

/** Entries a PR state change deserves: CI turned failing or passing, or the PR got merged. `prev` null is the first
 *  read, which only sets the baseline. A different PR number is a new baseline too. */
export const shipEvents = (prev: ShipStatus | null, next: ShipStatus, worktree: string, name: string): NewEntry[] => {
  if (!prev || prev.problem || next.problem || prev.number !== next.number) return [];
  const out: NewEntry[] = [];
  const pr = `#${next.number} ${next.title}`.trim();
  if (prev.pr_state === "open" && next.pr_state === "merged") out.push({ kind: "merged", worktree, text: L.notify.ship.merged(pr, name) });
  if (prev.ci !== next.ci && next.pr_state === "open") {
    if (next.ci === "fail") out.push({ kind: "ci-fail", worktree, text: L.notify.ship.ciFail(pr, name) });
    else if (next.ci === "pass" && prev.ci !== "none") out.push({ kind: "ci-pass", worktree, text: L.notify.ship.ciPass(pr, name) });
  }
  return out;
};

/** the palette (or anything else) asks the bell to open its list: the bell reacts to every change of this counter */
const [openRequest, setOpenRequest] = createSignal(0);
export { openRequest as notificationsOpenRequest };
export const requestNotificationsOpen = () => setOpenRequest((n) => n + 1);
