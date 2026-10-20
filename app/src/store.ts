import { createSignal } from "solid-js";
import { Changes, Commit, Project, SessionInfo, Worktree, gitChanges, listProjects, listSessions } from "./api";

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

/** bumped whenever the daemon reports file changes in the selected worktree */
export const [fsTick, setFsTick] = createSignal(0);

export const [projects, setProjects] = createSignal<Project[]>([]);
export const [sessions, setSessions] = createSignal<SessionInfo[]>([]);
export const [error, setError] = createSignal("");

const [wt, setWt] = createSignal<string | null>(load("axelstra.worktree"));
export const selectedWorktree = wt;
export const selectWorktree = (path: string | null) => {
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
export const [rightTab, setRightTab] = createSignal<"files" | "changes" | "review" | "pulls">("files");
/** the commit whose patch replaces the terminal area; null = none */
export const [openCommit, setOpenCommit] = createSignal<Commit | null>(null);
export const [openFile, setOpenFile] = createSignal<string | null>(null);

/** the project's kanban board replaces the terminal area when true */
export const [showKanban, setShowKanban] = createSignal(false);

/** a second terminal shown beside the active one (split view); null = no split */
export const [splitSession, setSplitSession] = createSignal<number | null>(null);

export const [activeSession, setActiveSession] = createSignal<number | null>(null);

/** "working": produced output in the last few seconds; "idle": quiet; "done": the process ended */
export const WORKING_MS = 3000;
export const sessionState = (s: SessionInfo): "working" | "idle" | "done" =>
  !s.running ? "done" : (s.idle_ms ?? Infinity) < WORKING_MS ? "working" : "idle";

/** worktrees whose agent finished while the user was looking elsewhere; cleared when selected */
export const [attention, setAttention] = createSignal<ReadonlySet<string>>(new Set());
const lastState = new Map<number, string>();

const notify = async (title: string, body: string) => {
  try {
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
    if (s.meta?.kind !== "agent" || !wtPath || prev !== "working" || state === "working") continue;
    const watching = !document.hidden && document.hasFocus() && norm(selectedWorktree()) === norm(wtPath);
    if (watching) continue;
    setAttention((cur) => new Set(cur).add(norm(wtPath)));
    const name = wtPath.split(/[\/]/).pop() ?? wtPath;
    void notify("Axelstra", state === "done" ? `${s.label ?? "agente"} terminou em ${name}` : `${s.label ?? "agente"} parou em ${name}`);
  }
};

export const refreshSessions = async () => {
  try {
    const list = await listSessions();
    trackAgents(list);
    setSessions(list);
    setError("");
  } catch (e) {
    setError(String(e));
  }
};

export const refreshProjects = async () => {
  try {
    setProjects(await listProjects());
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
