import { ask } from "@tauri-apps/plugin-dialog";
import { createSignal } from "solid-js";
import { Changes, Commit, GhItem, Project, SessionInfo, Worktree, WtStatus, ghListBg, gitChanges, listProjects, listSessions, worktreeStatus } from "./api";

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
const [agent, setAgentSig] = createSignal<string>(load("axelstra.agent") && AGENTS[load("axelstra.agent")!] ? load("axelstra.agent")! : "claude");
export const defaultAgent = agent;
export const setDefaultAgent = (a: string) => {
  if (!AGENTS[a]) return;
  setAgentSig(a);
  save("axelstra.agent", a);
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
/** errors set by an action stay until dismissed; only the session poll clears its own */
let errorFromPoll = false;
export const setError = (m: string) => {
  errorFromPoll = false;
  setErrorSig(m);
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
  setOpenDiff(null);
  setOpenFile(null);
  setOpenCommit(null);
  setShowKanban(false);
  setChanges(null);
};

/** right panel tab and the file open in the editor (relative to the worktree); null = terminal/diff */
export const [rightTab, setRightTab] = createSignal<"files" | "changes" | "pulls">("files");
/** the commit whose patch replaces the terminal area; null = none */
export const [openCommit, setOpenCommit] = createSignal<Commit | null>(null);
export const [openFile, setOpenFile] = createSignal<string | null>(null);

/** the project's kanban board replaces the terminal area when true */
export const [showKanban, setShowKanban] = createSignal(false);

/** a second terminal shown beside the active one (split view); null = no split */
export const [splitSession, setSplitSession] = createSignal<number | null>(null);

/** dev-server ports opened by the selected worktree's terminals */
export const [ports, setPorts] = createSignal<number[]>([]);

/** the automations dialog */
export const [showAutomations, setShowAutomations] = createSignal(false);
export const [showAgentSearch, setShowAgentSearch] = createSignal(false);

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

const notify = async (title: string, body: string, worktree: string, session: number) => {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    if (await invoke<boolean>("notify_session", { title, body, worktree, session })) return;
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
    const what = state === "done" ? "terminou" : state === "waiting" ? "precisa da tua resposta" : "parou";
    void notify("Axelstra", `${s.label ?? "agente"} ${what} em ${name}`, wtPath, s.id);
  }
};

export const refreshSessions = async () => {
  try {
    const list = await listSessions();
    trackAgents(list);
    setSessions(list);
    if (errorFromPoll) setErrorSig("");
  } catch (e) {
    setErrorSig(String(e));
    errorFromPoll = true;
  }
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
    setProjects(list);
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

export const refreshChanges = async () => {
  const w = selectedWorktree();
  if (!w || !findWorktree(w)) {
    setChanges(null);
    return;
  }
  try {
    setChanges(await gitChanges(w, baseRef()));
  } catch (e) {
    setChanges(null);
    setError(String(e));
  }
};
