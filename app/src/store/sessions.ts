import { createMemo, createRoot, createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { coded } from "../msg";
import { DispatchMeta, SessionInfo, callBg, flowTasks, listSessions, onDaemonEvent } from "../api";
import { errorText } from "../appError";
import { pushNotification } from "../notifications";
import { norm } from "../paths";
import { readString, writeString } from "../persist";
import { clearError, setError } from "./notify";
import { pruneTabState } from "./tabs";
import { guarded, keepSame, setSplitSession, showView } from "./ui";
import { setChanges } from "./git";
import { L } from "../strings.logic";
import { decideWait } from "../waitNotice";

export const [sessions, setSessions] = createSignal<SessionInfo[]>([]);

export const [activeSession, setActiveSession] = createSignal<number | null>(null);

/** "working": produced output in the last few seconds; "idle": quiet; "done": the process ended */
const WORKING_MS = 3000;
export const sessionState = (s: SessionInfo): "working" | "waiting" | "idle" | "done" =>
  !s.running ? "done" : s.waiting ? "waiting" : (s.idle_ms ?? Infinity) < WORKING_MS ? "working" : "idle";

/** worktrees whose agent finished while the user was looking elsewhere; cleared when selected */
export const [attention, setAttention] = createSignal<ReadonlySet<string>>(new Set());
const lastState = new Map<number, string>();
/** sessions the daemon already flagged as stalled, so each stall is announced once */
const stalledSeen = new Set<number>();
/** session -> the wait episode already announced (see decideWait) */
const waitSeen = new Map<number, string>();

/** opens a session from a clicked notification */
export const openSession = (path: string, id: number) =>
  guarded(() => {
    doSelect(path);
    setActiveSession(id);
  });

/** Counts every change of the selected worktree. A flow that selects something after an `await` takes `navStamp()`
 *  first and uses `selectIfStill`/`openIfStill`: if the user went elsewhere meanwhile, the view stays where they put it. */
let navEpoch = 0;
export const navStamp = () => navEpoch;
export const selectIfStill = (stamp: number, path: string | null) => {
  if (stamp === navEpoch) selectWorktree(path);
};
export const openIfStill = (stamp: number, path: string, id: number) => {
  if (stamp === navEpoch) openSession(path, id);
};

const notify = async (title: string, body: string, worktree: string, session: number, waiting: boolean) => {
  try {
    if (await invoke<boolean>("notify_session", { title, body, worktree, session, waiting })) return;
    const n = await import("@tauri-apps/plugin-notification");
    let ok = await n.isPermissionGranted();
    if (!ok) ok = (await n.requestPermission()) === "granted";
    if (ok) n.sendNotification({ title, body });
  } catch {
    /* notifications are a nicety; never break the UI over them */
  }
};

const appStart = Date.now();
/** when each session last started working; a restored session replays its screen for a moment and looks busy */
const workingSince = new Map<number, number>();
/** no notification for the first seconds after start (restored sessions settle) nor for a burst of work shorter than this */
const NOTIFY_GRACE_MS = 30000;
const NOTIFY_MIN_WORK_MS = 15000;

/** an agent that was working and went quiet needs a look: flag it and notify if the user is elsewhere */
const trackAgents = (list: SessionInfo[]) => {
  // sessions that were removed leave nothing behind
  const alive = new Set(list.map((s) => s.id));
  for (const id of lastState.keys()) if (!alive.has(id)) lastState.delete(id);
  for (const id of workingSince.keys()) if (!alive.has(id)) workingSince.delete(id);
  for (const id of stalledSeen) if (!alive.has(id)) stalledSeen.delete(id);
  for (const id of waitSeen.keys()) if (!alive.has(id)) waitSeen.delete(id);
  for (const s of list) {
    const state = sessionState(s);
    const prev = lastState.get(s.id);
    lastState.set(s.id, state);
    if (state === "working" && prev !== "working") workingSince.set(s.id, Date.now());
    const wtPath = s.meta?.worktree;
    if (s.meta?.kind !== "agent" || !wtPath) continue;
    // went quiet without reporting: one bell entry and one system notification per stall, whether or not it is on screen
    if (s.stalled && s.stall) {
      if (!stalledSeen.has(s.id)) {
        stalledSeen.add(s.id);
        const who = s.label ?? L.defaultAgent;
        const text = coded(s.stall.code, s.stall.params, s.stall.text);
        pushNotification({ kind: "waiting", worktree: wtPath, session: s.id, text });
        if (Date.now() - appStart >= NOTIFY_GRACE_MS)
          void notify(L.notify.stalledHead(who, wtPath.split(/[\\/]/).pop() ?? wtPath), text, wtPath, s.id, true);
      }
    } else stalledSeen.delete(s.id);
    // the agent asked for the user (permission or question): one bell entry and one system notification per wait
    {
      const viewing = !document.hidden && norm(selectedWorktree()) === norm(wtPath);
      const name = wtPath.split(/[\\/]/).pop() ?? wtPath;
      const d = decideWait(waitSeen.get(s.id), s.wait, s.label ?? L.defaultAgent, name, {
        grace: Date.now() - appStart < NOTIFY_GRACE_MS,
        watching: viewing && document.hasFocus(),
      });
      if (d.seen === undefined) waitSeen.delete(s.id);
      else waitSeen.set(s.id, d.seen);
      if (d.text) {
        if (!viewing) setAttention((cur) => new Set(cur).add(norm(wtPath)));
        pushNotification({ kind: "waiting", worktree: wtPath, session: s.id, text: d.text });
        if (d.os) void notify(L.notify.waiting.head(s.label ?? L.defaultAgent, name), d.text, wtPath, s.id, true);
      }
    }
    // back to work: the earlier "needs a look" flag is stale
    if (state === "working") {
      if (attention().has(norm(wtPath)))
        setAttention((cur) => {
          const n = new Set(cur);
          n.delete(norm(wtPath));
          return n;
        });
      continue;
    }
    if (prev !== "working") continue;
    // viewing: the worktree is on screen. watching: and the window has focus. document.hasFocus() is not
    // reliable inside the webview, so the dot only trusts "visible and selected"; focus only gates the notification.
    const viewing = !document.hidden && norm(selectedWorktree()) === norm(wtPath);
    const watching = viewing && document.hasFocus();
    if (!viewing) setAttention((cur) => new Set(cur).add(norm(wtPath)));
    // "waiting" is announced above, from the agent's event, not from here
    if (watching || state === "waiting") continue;
    const since = workingSince.get(s.id);
    if (Date.now() - appStart < NOTIFY_GRACE_MS || !since || Date.now() - since < NOTIFY_MIN_WORK_MS) continue;
    const name = wtPath.split(/[\\/]/).pop() ?? wtPath;
    const who = s.label ?? L.defaultAgent;
    const T = state === "done" ? L.notify.done : state === "waiting" ? L.notify.waiting : L.notify.stopped;
    const [head, body] = [T.head(who, name), T.body(who, name)];
    void notify(head, body, wtPath, s.id, state === "waiting");
    // a session that merely went quiet is not worth a line in the centre (it is the common case, and not an error)
    if (state === "done" || state === "waiting")
      pushNotification({ kind: state, worktree: wtPath, session: s.id, text: body });
  }
};

/** worktree (normalized path) -> its state in a running flow plan, for worktrees that have no session yet */
export const [flowState, setFlowState] = createSignal<Record<string, string>>({});
const refreshFlows = async () => {
  try {
    const out: Record<string, string> = {};
    const rank: Record<string, number> = { running: 4, queued: 3, failed: 2, blocked: 2, done: 1 };
    for (const t of await flowTasks()) {
      const k = norm(t.worktree);
      if (!out[k] || (rank[t.state] ?? 0) > (rank[out[k]] ?? 0)) out[k] = t.state;
    }
    setFlowState(out);
  } catch {
    /* an older daemon has no flow.list */
  }
};

/** A session as far as the UI can tell: idle_ms changes on every poll, but only its "working" side and the minute
 *  the labels show matter, so those stand in for it. */
const sessionSig = (s: SessionInfo) => ({
  ...s,
  idle_ms: s.idle_ms === undefined ? undefined : [s.idle_ms < WORKING_MS, Math.floor(s.idle_ms / 60000)],
});

const POLL_SESSIONS = "poll:sessions";
const pollSessions = async () => {
  void refreshFlows();
  try {
    const list = await listSessions();
    trackAgents(list);
    pruneTabState(list);
    // a worktree on screen is being looked at: a flag left from earlier (or set while the window was hidden) is stale
    const cur = selectedWorktree();
    if (cur && !document.hidden && attention().has(norm(cur)))
      setAttention((a) => {
        const n = new Set(a);
        n.delete(norm(cur));
        return n;
      });
    setSessions((prev) => keepSame(prev, list, (s) => s.id, sessionSig));
    clearError(POLL_SESSIONS);
  } catch (e) {
    setError(errorText(e), POLL_SESSIONS);
  }
};

/** One poll at a time. A call that arrives meanwhile shares one trailing poll (so it still sees data newer than
 *  the call), instead of piling up requests behind the slow one. */
let polling: Promise<void> | null = null;
let trailing: Promise<void> | null = null;
export const refreshSessions = (): Promise<void> => {
  if (!polling) return (polling = pollSessions().finally(() => (polling = null)));
  return (trailing ??= polling.then(() => {
    trailing = null;
    return refreshSessions();
  }));
};

/** titles the dispatcher gave each worktree, read once for everyone who shows them and again when the daemon says the file changed */
export const [dispatchTopics, setDispatchTopics] = createSignal<DispatchMeta>({});
const pollDispatch = () => callBg<DispatchMeta>("dispatch.meta").then(setDispatchTopics, () => {});
let dispUsers = 0;
let dispOff: (() => void) | undefined;
/** call from a component that shows `dispatchTopics`; returns the function that stops watching (the subscription ends with the last one) */
export const watchDispatch = () => {
  void pollDispatch();
  if (dispUsers++ === 0) dispOff = onDaemonEvent("dispatch.changed", () => void pollDispatch());
  return () => {
    if (--dispUsers === 0) dispOff?.();
  };
};

/** worktree (normalized path) -> its sessions, built once per change of `sessions` instead of once per row and call */
const sessionsByWorktree = createRoot(() =>
  createMemo(() => {
    const m = new Map<string, SessionInfo[]>();
    for (const s of sessions()) {
      const k = norm(s.meta?.worktree);
      if (!k) continue;
      const l = m.get(k);
      if (l) l.push(s);
      else m.set(k, [s]);
    }
    return m;
  }),
);
const NO_SESSIONS: SessionInfo[] = [];
export const sessionsOf = (path: string | null): SessionInfo[] =>
  path ? (sessionsByWorktree().get(norm(path)) ?? NO_SESSIONS) : NO_SESSIONS;

const [wt, setWt] = createSignal<string | null>(readString("axelstra.worktree") || null);
export const selectedWorktree = wt;
export const selectWorktree = (path: string | null) => guarded(() => doSelect(path));
const doSelect = (path: string | null) => {
  navEpoch++;
  setWt(path);
  if (path)
    setAttention((cur) => {
      const n = new Set(cur);
      n.delete(norm(path));
      return n;
    });
  writeString("axelstra.worktree", path ?? ""); // "" = none selected
  setActiveSession(null);
  setSplitSession(null);
  showView();
  setChanges(null);
};
