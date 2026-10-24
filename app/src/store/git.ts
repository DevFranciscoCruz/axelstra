import { createSignal } from "solid-js";
import { Changes, GhItem, Project, WtStatus, Worktree, ghListBg, gitChanges, listProjects, worktreeOverview, worktreeStatus } from "../api";
import { apiContext } from "../apiContext";
import { errorText } from "../appError";
import { norm } from "../paths";
import { createPoller, type Poller } from "../poller";
import type { OverviewProject } from "../statusGroups";
import { clearError, setError } from "./notify";
import { refreshSessions, selectedWorktree } from "./sessions";
import { keepSame, projectTerse } from "./ui";

export const [projects, setProjects] = createSignal<Project[]>([]);

const POLL_PROJECTS = "poll:projects";
const POLL_CHANGES = "poll:changes";

/** counts per project for the status bar and the sidebar: one poll for both, instead of one each */
export const [overview, setOverview] = createSignal<OverviewProject[]>([]);
export const refreshOverview = () =>
  worktreeOverview().then(
    (o) => {
      setOverview(o);
    },
    () => {
      /* the badges are a nicety: an old daemon without it, or one that is busy, just shows the last counts */
    },
  );
const OVERVIEW_MS = 15_000;
let overviewUsers = 0;
let overviewPoll: Poller | undefined;
/** call from a component that shows `overview`; returns the function that stops watching (the poll ends with the last one) */
export const watchOverview = () => {
  if (overviewUsers++ === 0) {
    void refreshOverview();
    overviewPoll = createPoller(refreshOverview, OVERVIEW_MS);
  }
  return () => {
    if (--overviewUsers === 0) overviewPoll?.stop();
  };
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
    clearError(POLL_PROJECTS);
  } catch (e) {
    setError(errorText(e), POLL_PROJECTS);
  }
};

/** "Tentar agora": any call makes the host reconnect to the daemon if it is down, so ask for the lists again */
export const retryDaemon = async () => {
  await Promise.allSettled([refreshSessions(), refreshProjects()]);
};

// api.ts reads projects and reply styles through this, so it does not have to import the store
apiContext.projects = projects;
apiContext.terse = projectTerse;

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
    clearError(POLL_CHANGES);
    return;
  }
  // a folder without git has no changes to list
  if (projects().find((p) => p.worktrees.some((x) => norm(x.path) === norm(w)))?.git === false) {
    setChanges(null);
    return;
  }
  try {
    const c = await gitChanges(w, baseRef());
    if (req === changesReq) {
      setChanges(c);
      clearError(POLL_CHANGES);
    }
  } catch (e) {
    if (req !== changesReq) return;
    setChanges(null);
    setError(errorText(e), POLL_CHANGES);
  }
};
