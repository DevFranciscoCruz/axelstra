import { batch, createMemo, createRoot, createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { coded } from "../msg";
import { DispatchMeta, SessionExit, SessionInfo, callBg, flowTasks, listSessions, onDaemonEvent } from "../api";
import { errorText } from "../appError";
import { pushNotification } from "../notifications";
import { mayInterrupt } from "../notifyPrefs";
import { norm } from "../paths";
import { readString, writeString } from "../persist";
import { clearError, setError } from "./notify";
import { pruneTabState, sortedTabs } from "./tabs";
import { toast } from "../toasts";
import { guarded, keepSame, setSplitSession, showView, splitSession } from "./ui";
import { setChanges } from "./git";
import { L } from "../strings.logic";
import { decideWait } from "../waitNotice";
import { autoSeatOf } from "../leadRole";

export const [sessions, setSessions] = createSignal<SessionInfo[]>([]);

/** false until the first `session.list` answered: an empty list before that says nothing about leads */
export const [sessionsLoaded, setSessionsLoaded] = createSignal(false);

const [active, setActive] = createSignal<number | null>(null);
export const activeSession = active;

/** Who asks for a change of the active session. Only the user changes it; `auto` is the app filling an empty seat. */
export type ActivateBy = "user" | "auto";

/** True while the app may seat a session by itself: at start (restoring) and right after the user selected a worktree or the
 *  daemon was replaced and restored the sessions. One automatic pick uses it up; a session that appears later (a spawn by an
 *  agent, a worker, dispatch) never finds it armed, so it cannot take the focus. */
let autoPick = true;
export const armAutoPick = () => {
  autoPick = true;
};

const seat = (id: number | null) => {
  // making the session shown in the split pane the active one swaps the two, so the pane never ends up empty
  if (id !== null && id === splitSession()) setSplitSession(active() !== id ? active() : null);
  setActive(id);
};

/** The ONE way to change the active session. `user`: always. `auto`: clearing (null) is always fine; a session only when the
 *  seat is empty and an automatic pick is armed (initial restore, a freshly selected worktree). Returns whether it applied. */
export const activate = (id: number | null, by: ActivateBy): boolean => {
  if (by === "auto" && id !== null) {
    if (!autoPick || active() !== null) return false;
    autoPick = false;
  }
  seat(id);
  return true;
};

/** Keeps the active session consistent with the tabs of the selected worktree (call from an effect). A worktree opens on its
 *  main (lead, or the project's main session), else on a terminal, never on a task (`autoSeatOf`), and only when an
 *  automatic pick is armed; a tab that vanished just empties the seat. It never switches from one live tab to another. */
export const settleActive = (tabs: SessionInfo[]) => {
  if (!sessionsLoaded()) return;
  const a = active();
  const missing = a === null || !tabs.some((s) => s.id === a);
  const pick = autoPick && missing ? autoSeatOf(tabs) : undefined;
  if (pick) {
    if (a !== null) setActive(null);
    activate(pick.id, "auto");
  } else if (a !== null && missing) activate(null, "auto");
  autoPick = false;
};

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
  // eslint-disable-next-line solid/reactivity -- only called from click handlers
  guarded(() => {
    doSelect(path);
    activate(id, "user");
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
    // `stalled` is the daemon saying nobody handles it (no live agent parent, or the parent did not resolve it): one bell
    // entry and one system notification per stall. `stall_parent` (a parent agent was told) is never an alert for the user.
    if (s.stalled && s.stall) {
      if (!stalledSeen.has(s.id)) {
        stalledSeen.add(s.id);
        const who = s.label ?? L.defaultAgent;
        const text = coded(s.stall.code, s.stall.params, s.stall.text);
        pushNotification({ kind: "waiting", worktree: wtPath, session: s.id, text, key: `stall:${s.id}` });
        if (Date.now() - appStart >= NOTIFY_GRACE_MS && mayInterrupt("need"))
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
        if (d.os && mayInterrupt("need"))
          void notify(L.notify.waiting.head(s.label ?? L.defaultAgent, name), d.text, wtPath, s.id, true);
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
    if (watching) continue;
    const since = workingSince.get(s.id);
    if (Date.now() - appStart < NOTIFY_GRACE_MS || !since || Date.now() - since < NOTIFY_MIN_WORK_MS) continue;
    // An agent that merely went quiet (a finished turn) is state, never a notification. Only a process that ended is "done";
    // one with a parent agent reports to it, so for the user that is informational.
    if (state !== "done") continue;
    const name = wtPath.split(/[\\/]/).pop() ?? wtPath;
    const who = s.label ?? L.defaultAgent;
    const [head, body] = [L.notify.done.head(who, name), L.notify.done.body(who, name)];
    const category = s.meta?.parent !== undefined ? "info" : "done";
    if (mayInterrupt(category)) void notify(head, body, wtPath, s.id, false);
    pushNotification({ kind: "done", worktree: wtPath, session: s.id, text: body, category });
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

/** Terminals that ended with an error. The daemon drops an ended session from `session.list` at once (`session.exited`),
 *  but the tab stays until the user closes it, so the error output can be read. Local to the app: never in `ls` or sessions.json. */
const kept = new Map<number, SessionInfo>();
/** exit code of each kept terminal, for its screen (the daemon no longer knows the session) */
const exitCodes = new Map<number, number>();
export const exitCodeOf = (id: number): number | undefined => exitCodes.get(id);

/** Sessions the daemon said ended, and when. A `session.list` that was already in flight must not bring their rows back;
 *  the entry expires because ids can come back after a daemon restart. */
const exited = new Map<number, number>();
const EXITED_TTL_MS = 10000;
const recentlyExited = (id: number) => {
  const at = exited.get(id);
  if (at === undefined) return false;
  if (Date.now() - at < EXITED_TTL_MS) return true;
  exited.delete(id);
  return false;
};

/** Where focus goes when the active tab goes away: the next tab of its worktree, else the one before; null when none is left. */
export const neighbourOf = (tabs: { id: number }[], gone: number, stays: (id: number) => boolean): number | null => {
  const at = tabs.findIndex((t) => t.id === gone);
  if (at < 0) return null;
  const after = tabs.slice(at + 1).find((t) => stays(t.id));
  const before = tabs
    .slice(0, at)
    .reverse()
    .find((t) => stays(t.id));
  return (after ?? before)?.id ?? null;
};

/** The tab the user is on is leaving the list: focus moves to a neighbour. A tab that is not the active one never moves focus. */
const refocusAfterDrop = (prev: SessionInfo[], next: SessionInfo[]) => {
  const a = activeSession();
  if (a !== null && !next.some((s) => s.id === a)) {
    const was = prev.find((s) => s.id === a);
    if (was) {
      const k = norm(was.meta?.worktree);
      const here = sortedTabs(prev.filter((s) => norm(s.meta?.worktree) === k));
      // rule: only the active tab leaving moves focus, once, to its neighbour
      seat(neighbourOf(here, a, (id) => next.some((s) => s.id === id)));
    }
  }
  const sp = splitSession();
  if (sp !== null && !next.some((s) => s.id === sp)) setSplitSession(null);
};

/** takes the rows of these sessions off the list now (their tabs close), moving focus only if the active one is among them */
const dropSessions = (ids: number[]) => {
  const prev = sessions();
  const next = prev.filter((s) => !ids.includes(s.id));
  if (next.length === prev.length) return;
  refocusAfterDrop(prev, next);
  setSessions(next);
};

/** the user closed the tab of an ended terminal (or the session is gone for good) */
export const dismissExited = (id: number) => {
  exitCodes.delete(id);
  if (kept.delete(id)) dropSessions([id]);
};

/** The daemon does not know this session any more (a terminal got "no such session"): its tab goes, unless it is a failed
 *  terminal the app keeps on purpose. */
export const sessionGone = (id: number) => {
  if (kept.has(id)) return;
  exited.set(id, Date.now());
  dropSessions([id]);
  void refreshSessions();
};

/** The daemon says a session's program ended by itself and it already left the list. Same notifications as when the poll
 *  used to see it stop; a non-zero code is shown. The tab of an agent or of a terminal that ended well closes at once;
 *  only a failed terminal keeps it (and its code) until the user closes it. */
export const handleSessionExited = ({ id, code }: SessionExit) => {
  const prev = sessions().find((s) => s.id === id);
  if (!prev || kept.has(id)) return;
  const gone: SessionInfo = { ...prev, running: false, waiting: false, wait: null, stalled: false, stall: null };
  trackAgents(sessions().map((s) => (s.id === id ? gone : s)));
  if (code != null && code !== 0) {
    toast({ text: L.notify.exitCode(prev.label ?? L.session(id), code), kind: "error", key: `exit:${id}` });
    if (prev.meta?.kind !== "agent") {
      kept.set(id, gone);
      exitCodes.set(id, code);
    }
  }
  if (!kept.has(id)) {
    exited.set(id, Date.now());
    dropSessions([id]);
  }
  void refreshSessions();
};

const POLL_SESSIONS = "poll:sessions";
const pollSessions = async () => {
  void refreshFlows();
  try {
    const list = (await listSessions()).filter((s) => !recentlyExited(s.id));
    // the daemon lists an id again (a restart handed it to a new session): the kept row is not that session
    for (const id of [...kept.keys()])
      if (list.some((x) => x.id === id)) {
        kept.delete(id);
        exitCodes.delete(id);
      }
    list.push(...kept.values());
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
    // an id that is no longer listed has no tab, no focus and no split pane (also right after a restore or a daemon restart)
    refocusAfterDrop(sessions(), list);
    setSessions((prev) => keepSame(prev, list, (s) => s.id, sessionSig));
    setSessionsLoaded(true);
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
  // one batch, armed first: the effect that seats a tab must see the new worktree together with an armed pick, never the new
  // worktree with a pick an earlier empty settle already used up (then nothing would seat a tab and no terminal would mount)
  batch(() => {
    setActive(null);
    armAutoPick();
    setWt(path);
    if (path)
      setAttention((cur) => {
        const n = new Set(cur);
        n.delete(norm(path));
        return n;
      });
    writeString("axelstra.worktree", path ?? ""); // "" = none selected
    setSplitSession(null);
    showView();
    setChanges(null);
  });
};
