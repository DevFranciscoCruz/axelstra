import { ask } from "./Confirm";
import { createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { Changes, Commit, DispatchMeta, FlowTask, GhItem, Project, SessionInfo, Terse, Worktree, WtStatus, call, callBg, flowTasks, ghListBg, gitChanges, listProjects, listSessions, worktreeStatus } from "./api";

/** Paths come back with either slash style; compare them in one normal form. */
export const norm = (p?: string | null) => (p ?? "").replace(/\\/g, "/").replace(/\/$/, "").toLowerCase();

const load = (k: string): string | null => {
  try {
    return localStorage.getItem(k);
  } catch {
    return null;
  }
};
const save = (k: string, v: string | null) => {
  try {
    if (v === null) localStorage.removeItem(k);
    else localStorage.setItem(k, v);
  } catch {
    /* storage can be unavailable; the selection just is not remembered */
  }
};

/** CLI agents that can be launched; the key is what the UI shows and stores */
export const AGENTS: Record<string, string> = { claude: "claude", chatgpt: "codex" };
/** names of the agents the pickers list: the built-in two plus any other agent CLI found on the PATH */
export const [agentKeys, setAgentKeys] = createSignal<string[]>(Object.keys(AGENTS));
export const addAgents = (names: string[]) => {
  for (const n of names) AGENTS[n] = n;
  setAgentKeys(Object.keys(AGENTS));
};
const [agent, setAgentSig] = createSignal<string>(load("axelstra.agent") && AGENTS[load("axelstra.agent")!] ? load("axelstra.agent")! : "claude");
export const defaultAgent = agent;
export const setDefaultAgent = (a: string) => {
  if (!AGENTS[a]) return;
  setAgentSig(a);
  save("axelstra.agent", a);
};

/** Settings that belong to one project, remembered in the browser storage next to the global ones. */
export type ProjectPrefs = { agent?: string; terse?: Terse };
const loadPrefs = (): Record<string, ProjectPrefs> => {
  try {
    const v = JSON.parse(load("axelstra.projectPrefs") ?? "{}");
    return v && typeof v === "object" && !Array.isArray(v) ? v : {};
  } catch {
    return {};
  }
};
const [projectPrefs, setProjectPrefsSig] = createSignal<Record<string, ProjectPrefs>>(loadPrefs());
/** the agent a project opens by default: its own choice, else the global one */
export const projectAgent = (id?: string | null) => {
  const a = id ? projectPrefs()[id]?.agent : undefined;
  return a && AGENTS[a] ? a : defaultAgent();
};
export const projectPrefsOf = (id: string): ProjectPrefs => projectPrefs()[id] ?? {};
/** reply style chosen for one project; undefined = follow the global setting */
export const projectTerse = (id?: string | null): Terse | undefined => (id ? projectPrefs()[id]?.terse : undefined);
/** changes one project's preference; `undefined` for a field removes it (back to the global value) */
export const setProjectPref = (id: string, patch: ProjectPrefs) => {
  const next = { ...projectPrefs() };
  const merged: ProjectPrefs = { ...next[id], ...patch };
  for (const k of Object.keys(merged) as (keyof ProjectPrefs)[]) if (merged[k] === undefined) delete merged[k];
  if (Object.keys(merged).length) next[id] = merged;
  else delete next[id];
  setProjectPrefsSig(next);
  save("axelstra.projectPrefs", JSON.stringify(next));
};

/** Modal asking which agent to open. Resolves with the agent key, "shell" for a plain terminal, or null if dismissed. */
export const [agentPrompt, setAgentPrompt] = createSignal<((a: string | null) => void) | null>(null);
export const pickAgent = () =>
  new Promise<string | null>((resolve) => {
    setAgentPrompt(() => (a: string | null) => {
      setAgentPrompt(null);
      resolve(a);
    });
  });

/** left project list and right files/changes panel; remembered between runs */
const flag = (k: string) => load(k) !== "0";
const [leftOn, setLeftOn] = createSignal(flag("axelstra.left"));
const [rightOn, setRightOn] = createSignal(flag("axelstra.right"));
export const showSidebar = leftOn;
export const showRightPanel = rightOn;
export const toggleSidebar = () => {
  setLeftOn(!leftOn());
  save("axelstra.left", leftOn() ? "1" : "0");
};
export const toggleRightPanel = () => {
  setRightOn(!rightOn());
  save("axelstra.right", rightOn() ? "1" : "0");
};

/** panel widths in px (drag handles); remembered, clamped on use */
const num = (k: string, d: number) => {
  const n = Number(load(k));
  return Number.isFinite(n) && n > 0 ? n : d;
};
export const [leftW, setLeftWSig] = createSignal(num("axelstra.leftW", 260));
export const [rightW, setRightWSig] = createSignal(num("axelstra.rightW", 320));
export const setLeftW = (w: number) => {
  const v = Math.round(Math.min(Math.max(w, 180), 480));
  setLeftWSig(v);
  save("axelstra.leftW", String(v));
};
export const setRightW = (w: number) => {
  const v = Math.round(Math.min(Math.max(w, 240), 640));
  setRightWSig(v);
  save("axelstra.rightW", String(v));
};

/** unsaved text in the open editor; anything that would close it asks first */
export const [editorDirty, setEditorDirty] = createSignal(false);
export const confirmDiscard = async () =>
  !editorDirty() ||
  (await ask("Há alterações por guardar neste ficheiro. Descartá-las?", { title: "Editor", kind: "warning", okLabel: "Descartar", cancelLabel: "Cancelar" }));
/** runs `fn` now, or after the user agrees to drop unsaved editor text */
export const guarded = (fn: () => void) => {
  if (!editorDirty()) return fn();
  void confirmDiscard().then((ok) => {
    if (ok) {
      setEditorDirty(false);
      fn();
    }
  });
};

/** bumped whenever the daemon reports file changes in the selected worktree */
export const [fsTick, setFsTick] = createSignal(0);

export const [projects, setProjects] = createSignal<Project[]>([]);
export const [sessions, setSessions] = createSignal<SessionInfo[]>([]);
const [error, setErrorSig] = createSignal("");
export { error };
/** the app is reconnecting to the daemon (it was replaced or crashed): the UI says so until it is back */
export const [daemonReconnecting, setDaemonReconnecting] = createSignal(false);
/** a short informational message (not a failure): shown for a few seconds */
export const [notice, setNoticeSig] = createSignal("");
let noticeTimer: ReturnType<typeof setTimeout> | undefined;
export const showNotice = (m: string) => {
  setNoticeSig(m);
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => setNoticeSig(""), 9000);
};

/** errors set by an action stay until dismissed; only the session poll clears its own */
let errorFromPoll = false;
export const setError = (m: string) => {
  errorFromPoll = false;
  setErrorSig(friendlyError(m));
};

/** the daemon and git speak English; the common failures get a sentence that says what to do */
const FRIENDLY: [RegExp, string][] = [
  [/os error 2|program not found|not recognized|cannot find the file/i, "Programa não encontrado. Confirma que está instalado e no PATH (git, claude, codex ou gh)."],
  [/is already checked out|already exists/i, "Já existe um branch ou worktree com esse nome. Escolhe outro nome."],
  [/not a git repository/i, "Esta pasta não é um repositório git."],
  [/contains modified or untracked files|local changes would be overwritten/i, "Há alterações por guardar. Faz commit ou descarta-as primeiro."],
  [/could not read from remote|authentication failed|permission denied \(publickey\)|could not resolve host/i, "Sem acesso ao remoto. Verifica a ligação à internet e o login no git/gh."],
  [/non-fast-forward|failed to push|rejected/i, "O remoto recusou o push porque tem commits novos. Faz pull antes de enviar."],
  [/worker limit reached/i, "Limite de agentes em simultâneo atingido. Espera que um termine ou aumenta com `axelstra limit`."],
  [/connection (refused|reset)|pipe.*(closed|broken)|os error 232|os error 109/i, "Sem ligação ao daemon. Vai tentar de novo; se persistir, reinicia o Axelstra."],
  [/unknown method/i, "O daemon em execução é antigo. Reinicia o Axelstra para o atualizar."],
];
export const friendlyError = (m: string) => {
  const hit = FRIENDLY.find(([re]) => re.test(m));
  return hit ? `${hit[1]}\n(${m.length > 160 ? m.slice(0, 160) + "…" : m})` : m;
};

const [wt, setWt] = createSignal<string | null>(load("axelstra.worktree"));
export const selectedWorktree = wt;
export const selectWorktree = (path: string | null) => guarded(() => doSelect(path));
const doSelect = (path: string | null) => {
  setWt(path);
  if (path) setAttention((cur) => { const n = new Set(cur); n.delete(norm(path)); return n; });
  save("axelstra.worktree", path);
  setActiveSession(null);
  setSplitSession(null);
  showView();
  setChanges(null);
};

/** right panel tab and the file open in the editor (relative to the worktree); null = terminal/diff */
export const [rightTab, setRightTab] = createSignal<"files" | "changes" | "pulls">("files");
/** the commit whose patch replaces the terminal area; null = none */
export const [openCommit, setOpenCommit] = createSignal<Commit | null>(null);
export const [openFile, setOpenFile] = createSignal<string | null>(null);
/** line to put the cursor on when the open file was reached from a path link; null = top */
export const [fileLine, setFileLine] = createSignal<number | null>(null);

/** the project's kanban board replaces the terminal area when true */
export const [showKanban, setShowKanban] = createSignal(false);

/** One place that picks what fills the main area: no argument = the terminal; the views exclude each other. */
export const showView = (v: { diff?: string; file?: string; commit?: Commit; kanban?: boolean } = {}) => {
  setOpenDiff(v.diff ?? null);
  setOpenFile(v.file ?? null);
  setFileLine(null);
  setOpenCommit(v.commit ?? null);
  setShowKanban(v.kanban ?? false);
};
/** opens a file (relative to the worktree) in the editor, at `line` when given; asks first if the editor has unsaved text */
export const openFileAt = (path: string, line?: number) =>
  guarded(() => {
    showView({ file: path });
    setFileLine(line && line > 0 ? line : null);
  });

/** a second terminal shown beside the active one (split view); null = no split */
export const [splitSession, setSplitSession] = createSignal<number | null>(null);

/** dev-server ports opened by the selected worktree's terminals */
export const [ports, setPorts] = createSignal<number[]>([]);

/** project whose "new worktree" dialog is open: the sidebar shows it, the command palette can ask for it */
export const [creatingWorktree, setCreatingWorktree] = createSignal<string | null>(null);
/** a worktree the command palette asked to remove; the sidebar confirms and does it */
export const [removeRequest, setRemoveRequest] = createSignal<{ project: string; path: string; name: string } | null>(null);

/** the automations dialog */
export const [showAutomations, setShowAutomations] = createSignal(false);
export const [showSettings, setShowSettings] = createSignal(false);
export const [showDispatcher, setShowDispatcher] = createSignal(false);
export const [showShortcuts, setShowShortcuts] = createSignal(false);
export const [showAgentSearch, setShowAgentSearch] = createSignal(false);
export const [showOrchestra, setShowOrchestra] = createSignal(false);
/** agents may look at the screen and drive the mouse and keyboard (held by the daemon, off at every start) */
export const [computerOn, setComputerOn] = createSignal(false);

/** worktrees pinned to the top of their project (remembered in the browser storage) */
const loadPins = (): string[] => {
  try {
    return JSON.parse(load("axelstra.pins") ?? "[]");
  } catch {
    return [];
  }
};
const [pins, setPins] = createSignal<string[]>(loadPins());
export const isPinned = (path: string) => pins().includes(norm(path));
export const togglePin = (path: string) => {
  const n = norm(path);
  const next = pins().includes(n) ? pins().filter((x) => x !== n) : [...pins(), n];
  setPins(next);
  save("axelstra.pins", JSON.stringify(next));
};

export const [activeSession, setActiveSession] = createSignal<number | null>(null);

/** "working": produced output in the last few seconds; "idle": quiet; "done": the process ended */
export const WORKING_MS = 3000;
export const sessionState = (s: SessionInfo): "working" | "waiting" | "idle" | "done" =>
  !s.running ? "done" : s.waiting ? "waiting" : (s.idle_ms ?? Infinity) < WORKING_MS ? "working" : "idle";

/** worktrees whose agent finished while the user was looking elsewhere; cleared when selected */
export const [attention, setAttention] = createSignal<ReadonlySet<string>>(new Set());
const lastState = new Map<number, string>();

/** opens a session from a clicked notification */
export const openSession = (path: string, id: number) =>
  guarded(() => {
    doSelect(path);
    setActiveSession(id);
  });

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

/** an agent that was working and went quiet needs a look: flag it and notify if the user is elsewhere */
const trackAgents = (list: SessionInfo[]) => {
  // sessions that were removed leave nothing behind
  const alive = new Set(list.map((s) => s.id));
  for (const id of lastState.keys()) if (!alive.has(id)) lastState.delete(id);
  for (const s of list) {
    const state = sessionState(s);
    const prev = lastState.get(s.id);
    lastState.set(s.id, state);
    const wtPath = s.meta?.worktree;
    if (s.meta?.kind !== "agent" || !wtPath) continue;
    // back to work: the earlier "needs a look" flag is stale
    if (state === "working") {
      if (attention().has(norm(wtPath))) setAttention((cur) => { const n = new Set(cur); n.delete(norm(wtPath)); return n; });
      continue;
    }
    if (prev !== "working") continue;
    // viewing: the worktree is on screen. watching: and the window has focus. document.hasFocus() is not
    // reliable inside the webview, so the dot only trusts "visible and selected"; focus only gates the notification.
    const viewing = !document.hidden && norm(selectedWorktree()) === norm(wtPath);
    const watching = viewing && document.hasFocus();
    if (!viewing) setAttention((cur) => new Set(cur).add(norm(wtPath)));
    if (watching) continue;
    const name = wtPath.split(/[\/]/).pop() ?? wtPath;
    const who = s.label ?? "O agente";
    const [head, body] =
      state === "done"
        ? [`${who} terminou · ${name}`, `${who} acabou a tarefa na worktree "${name}". Abre para rever o resultado.`]
        : state === "waiting"
          ? [`${who} precisa de ti · ${name}`, `${who} fez uma pergunta na worktree "${name}" e precisa da tua resposta para continuar.`]
          : [`${who} parou · ${name}`, `${who} ficou parado na worktree "${name}" sem terminar. Abre para ver o que aconteceu.`];
    void notify(head, body, wtPath, s.id, state === "waiting");
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

/** The new list, but with the old object wherever nothing changed: keyed lists then keep their DOM (tabs keep focus,
 *  icons do not reload) instead of being rebuilt on every poll. `sig` is what counts as a change (default: everything);
 *  pass one that leaves out fields that move on every poll. */
export const keepSame = <T>(prev: T[], next: T[], key: (x: T) => string | number, sig: (x: T) => unknown = (x) => x): T[] => {
  const old = new Map(prev.map((x) => [key(x), x] as const));
  return next.map((n) => {
    const o = old.get(key(n));
    return o !== undefined && JSON.stringify(sig(o)) === JSON.stringify(sig(n)) ? o : n;
  });
};

/** A session as far as the UI can tell: idle_ms changes on every poll, but only its "working" side and the minute
 *  the labels show matter, so those stand in for it. */
const sessionSig = (s: SessionInfo) => ({ ...s, idle_ms: s.idle_ms === undefined ? undefined : [s.idle_ms < WORKING_MS, Math.floor(s.idle_ms / 60000)] });

const pollSessions = async () => {
  void refreshFlows();
  try {
    const list = await listSessions();
    trackAgents(list);
    pruneTabState(list);
    // a worktree on screen is being looked at: a flag left from earlier (or set while the window was hidden) is stale
    const cur = selectedWorktree();
    if (cur && !document.hidden && attention().has(norm(cur))) setAttention((a) => { const n = new Set(a); n.delete(norm(cur)); return n; });
    setSessions((prev) => keepSame(prev, list, (s) => s.id, sessionSig));
    if (errorFromPoll) setErrorSig("");
  } catch (e) {
    setErrorSig(String(e));
    errorFromPoll = true;
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

/** titles the dispatcher gave each worktree, polled once for everyone who shows them */
export const [dispatchTopics, setDispatchTopics] = createSignal<DispatchMeta>({});
const pollDispatch = () => callBg<DispatchMeta>("dispatch.meta").then(setDispatchTopics, () => {});
let dispUsers = 0;
let dispTimer: ReturnType<typeof setInterval> | undefined;
/** call from a component that shows `dispatchTopics`; returns the function that stops watching (the poll ends with the last one) */
export const watchDispatch = () => {
  void pollDispatch();
  if (dispUsers++ === 0) dispTimer = setInterval(() => !document.hidden && void pollDispatch(), 4000);
  return () => {
    if (--dispUsers === 0) clearInterval(dispTimer);
  };
};

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
  void call("session.rename", { id, label: n }).then(() => refreshSessions()).catch(() => {});
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
const pruneTabState = (list: SessionInfo[]) => {
  const alive = new Set(list.map((s) => s.id));
  if (Object.keys(tabNames()).some((k) => !alive.has(Number(k)))) setTabNames((cur) => Object.fromEntries(Object.entries(cur).filter(([k]) => alive.has(Number(k)))));
  if (tabOrder().some((id) => !alive.has(id))) setTabOrder((cur) => cur.filter((id) => alive.has(id)));
};

/** git state per worktree (keyed by normalized path) and the open PR per branch, for the sidebar badges */
export const [wtStatus, setWtStatus] = createSignal<Record<string, WtStatus>>({});
export const [prByBranch, setPrByBranch] = createSignal<Record<string, GhItem>>({});
export const prKey = (project: string, branch: string | null | undefined) => `${project}:${branch ?? ""}`;
let lastPrs = 0;

const refreshWtStatus = async (list: Project[]) => {
  const paths = list.flatMap((p) => p.worktrees.map((w) => w.path));
  if (!paths.length) return;
  try {
    const out: Record<string, WtStatus> = {};
    for (const st of await worktreeStatus(paths)) out[norm(st.path)] = st;
    setWtStatus(out);
  } catch {
    /* badges are a nicety; an old daemon without worktree.status just shows none */
  }
};

/** gh is slow and rate limited: at most every 3 minutes, and only for projects that have a remote */
const refreshPrs = async (list: Project[]) => {
  if (Date.now() - lastPrs < 180_000) return;
  lastPrs = Date.now();
  const out: Record<string, GhItem> = {};
  for (const p of list) {
    if (!p.remote) continue;
    try {
      for (const it of await ghListBg(p.id, "pr")) if (it.branch) out[prKey(p.id, it.branch)] = it;
    } catch {
      /* gh missing or not logged in: no PR badges */
    }
  }
  setPrByBranch(out);
};

export const refreshProjects = async () => {
  try {
    const list = await listProjects();
    setProjects((prev) => keepSame(prev, list, (p) => p.id));
    void refreshWtStatus(list);
    void refreshPrs(list);
  } catch (e) {
    setError(String(e));
  }
};

export const sessionsOf = (path: string | null) =>
  path ? sessions().filter((s) => norm(s.meta?.worktree) === norm(path)) : [];

export const findWorktree = (path: string | null): { project: Project; wt: Worktree } | null => {
  if (!path) return null;
  for (const project of projects()) {
    const w = project.worktrees.find((x) => norm(x.path) === norm(path));
    if (w) return { project, wt: w };
  }
  return null;
};

// ---- changes panel ----
export const [changes, setChanges] = createSignal<Changes | null>(null);
/** "head": uncommitted work. "main": everything since the branch left the main worktree's branch. */
export const [changesMode, setChangesMode] = createSignal<"head" | "main">("head");
/** which file's diff is showing instead of a terminal; null = terminal */
export const [openDiff, setOpenDiff] = createSignal<string | null>(null);

export const baseRef = (): string | undefined => {
  if (changesMode() !== "main") return undefined;
  const c = findWorktree(selectedWorktree());
  return c?.project.worktrees.find((w) => w.is_main)?.branch ?? undefined;
};

/** newest refreshChanges call; an answer that is no longer the newest (other worktree, other mode) is dropped */
let changesReq = 0;
export const refreshChanges = async () => {
  const req = ++changesReq;
  const w = selectedWorktree();
  if (!w || !findWorktree(w)) {
    setChanges(null);
    return;
  }
  // a folder without git has no changes to list
  if (projects().find((p) => p.worktrees.some((x) => norm(x.path) === norm(w)))?.git === false) {
    setChanges(null);
    return;
  }
  try {
    const c = await gitChanges(w, baseRef());
    if (req === changesReq) setChanges(c);
  } catch (e) {
    if (req !== changesReq) return;
    setChanges(null);
    setError(String(e));
  }
};
