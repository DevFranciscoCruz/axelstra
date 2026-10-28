import { createSignal } from "solid-js";
import {
  Changes,
  GhItem,
  Project,
  WtStatus,
  Worktree,
  ghListBg,
  gitChanges,
  listProjects,
  prRefresh,
  worktreeOverview,
  worktreeStatus,
} from "../api";
import { apiContext } from "../apiContext";
import { errorText } from "../appError";
import { norm } from "../paths";
import { createLive, type Live } from "../live";
import { coalesced } from "../requestGate";
import type { OverviewProject } from "../statusGroups";
import { clearError, setError } from "./notify";
import { refreshSessions, selectedWorktree } from "./sessions";
import { showRightPanel } from "./layout";
import { keepSame, openDiff, projectTerse } from "./ui";
import { createEffect, createRoot } from "solid-js";

export const [projects, setProjects] = createSignal<Project[]>([]);
/** true once the first `listProjects` answered (ok or not): until then an empty list means "not known yet", not "first run" */
export const [projectsLoaded, setProjectsLoaded] = createSignal(false);
/** the last `listProjects` failed (daemon down or silent): an empty list then says nothing about the user's projects */
export const [projectsFailed, setProjectsFailed] = createSignal(false);

const POLL_PROJECTS = "poll:projects";
const POLL_CHANGES = "poll:changes";

/** counts per project for the status bar and the sidebar: one poll for both, instead of one each */
export const [overview, setOverview] = createSignal<OverviewProject[]>([]);
/** one overview in flight at a time (events during a run cause one more run after it): it runs git over every worktree */
export const refreshOverview = coalesced(() =>
  worktreeOverview().then(
    (o) => {
      setOverview(o);
    },
    () => {
      /* the badges are a nicety: an old daemon without it, or one that is busy, just shows the last counts */
    },
  ),
);
let overviewUsers = 0;
let overviewLive: Live | undefined;
let prLive: Live | undefined;
/** the window got focus: the daemon looks once at the PRs it follows (GitHub changes raise no local event); `pr.changed` follows if one moved */
const askPrRefresh = () => void Promise.resolve(prRefresh()).catch(() => undefined);
/** call from a component that shows `overview`; returns the function that stops watching (the subscription ends with the last one) */
export const watchOverview = () => {
  if (overviewUsers++ === 0) {
    // a PR moved: the PR badges of the sidebar are read again (their 3 minute limit only protects gh from bursts)
    prLive = createLive(
      () => {
        lastPrs = 0;
        return refreshPrs(projects());
      },
      ["pr.changed"],
      { onFocus: false },
    );
    if (typeof window !== "undefined") window.addEventListener("focus", askPrRefresh);
    // the counts move with worktrees, sessions, the dispatcher, the janitor and PRs: the daemon says so
    overviewLive = createLive(refreshOverview, [
      "projects.changed",
      "session.changed",
      "dispatch.changed",
      "janitor.events",
      "pr.changed",
    ]);
    overviewLive.now();
  }
  return () => {
    if (--overviewUsers === 0) {
      overviewLive?.stop();
      prLive?.stop();
      if (typeof window !== "undefined") window.removeEventListener("focus", askPrRefresh);
    }
  };
};

/** git state per worktree (keyed by normalized path) and the open PR per branch, for the sidebar badges */
export const [wtStatus, setWtStatus] = createSignal<Record<string, WtStatus>>({});
export const [prByBranch, setPrByBranch] = createSignal<Record<string, GhItem>>({});
export const prKey = (project: string, branch: string | null | undefined) => `${project}:${branch ?? ""}`;
let lastPrs = 0;

/** `worktree.status` runs git in every worktree: one at a time, at most one per 2 s, always over the newest project list */
// eslint-disable-next-line solid/reactivity -- reads the project list when it runs, not to track it
const refreshWtStatus = coalesced(async () => {
  if (typeof document !== "undefined" && document.hidden) return;
  const paths = projects().flatMap((p) => p.worktrees.map((w) => w.path));
  if (!paths.length) return;
  try {
    const out: Record<string, WtStatus> = {};
    for (const st of await worktreeStatus(paths)) out[norm(st.path)] = st;
    setWtStatus(out);
  } catch {
    /* badges are a nicety; an old daemon without worktree.status just shows none */
  }
}, 2000);

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

/** `project.list` calls made together (focus, watch event, a sidebar action) share one run; a call during a run causes one more after it */
export const refreshProjects = coalesced(async () => {
  try {
    const list = await listProjects();
    setProjects((prev) => keepSame(prev, list, (p) => p.id));
    void refreshWtStatus();
    void refreshPrs(list);
    setProjectsFailed(false);
    setProjectsLoaded(true);
    clearError(POLL_PROJECTS);
  } catch (e) {
    setProjectsFailed(true);
    setProjectsLoaded(true);
    setError(errorText(e), POLL_PROJECTS);
  }
});

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
/** the change list was asked for while the window was hidden: it is read once when the window shows again */
let changesStale = false;
const loadChanges = async () => {
  if ((typeof document !== "undefined" && document.hidden) || (!showRightPanel() && !openDiff())) {
    changesStale = true;
    return;
  }
  changesStale = false;
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

/** `git.changes` is the slowest read: one at a time per worktree and mode (another worktree does not wait), at most one per 300 ms, nothing while hidden */
const changesGates = new Map<string, ReturnType<typeof coalesced<void>>>();
export const refreshChanges = () => {
  const key = `${norm(selectedWorktree() ?? "")}|${baseRef() ?? ""}`;
  let gate = changesGates.get(key);
  if (!gate) {
    gate = coalesced(loadChanges, 300);
    changesGates.set(key, gate);
  }
  return gate();
};
if (typeof document !== "undefined")
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && changesStale) void refreshChanges();
  });
// the right panel was opened (or a diff tab shown) while the list was stale: read it now
createRoot(() =>
  createEffect(() => {
    if ((showRightPanel() || openDiff()) && changesStale) void refreshChanges();
  }),
);
