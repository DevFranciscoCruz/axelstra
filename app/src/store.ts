import { createSignal } from "solid-js";
import { Changes, Project, SessionInfo, Worktree, gitChanges, listProjects, listSessions } from "./api";

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

export const [projects, setProjects] = createSignal<Project[]>([]);
export const [sessions, setSessions] = createSignal<SessionInfo[]>([]);
export const [error, setError] = createSignal("");

const [wt, setWt] = createSignal<string | null>(load("axelstra.worktree"));
export const selectedWorktree = wt;
export const selectWorktree = (path: string | null) => {
  setWt(path);
  save("axelstra.worktree", path);
  setActiveSession(null);
  setOpenDiff(null);
  setChanges(null);
};

export const [activeSession, setActiveSession] = createSignal<number | null>(null);

export const refreshSessions = async () => {
  try {
    setSessions(await listSessions());
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
