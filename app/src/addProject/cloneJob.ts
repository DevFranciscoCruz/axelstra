import { createSignal } from "solid-js";
import {
  type CloneParams,
  type CloneState,
  type CloneStatus,
  projectClone,
  projectCloneCancel,
  projectCloneStatus,
} from "../api";
import { errorText } from "../appError";
import { coded } from "../msg";
import {
  navStamp,
  projects,
  refreshProjects,
  refreshSessions,
  selectIfStill,
  setError,
  setShowAddProject,
  showAddProject,
  showNotice,
} from "../store";
import { AP } from "../strings.sources";
import { repoNameFromUrl } from "./logic";

/** How often the daemon is asked how far the clone is. */
export const POLL_MS = 500;
/** consecutive failed polls before the job is given up on */
const MAX_POLL_ERRORS = 3;

export type CloneJob = {
  /** "starting" until the daemon answered `project.clone` with an id */
  state: CloneState | "starting";
  id: string;
  dest: string;
  name: string;
  percent?: number;
  phase?: string;
  /** readable text of why it failed */
  error?: string;
};

/** The one clone in flight (or the last one's result). Module-level on purpose: closing the dialog does not stop the clone, and its end is still reported. */
export const [cloneJob, setCloneJob] = createSignal<CloneJob | null>(null);

export const cloneActive = (j: CloneJob | null = cloneJob()): boolean =>
  !!j && (j.state === "starting" || j.state === "running");

let gen = 0;
let cancelAsked = false;

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const patch = (mine: number, p: Partial<CloneJob>) => {
  if (mine === gen) setCloneJob((j) => (j ? { ...j, ...p } : j));
};

/** the end of a job: refresh the projects and open the new one, or say why not; a toast only when the dialog is no longer there to show it */
const finish = async (mine: number, st: CloneStatus) => {
  const j = cloneJob();
  if (mine !== gen || !j) return;
  if (st.state === "done") {
    const stamp = navStamp();
    patch(mine, { state: "done", percent: 100 });
    await refreshProjects();
    const added = projects().find((p) => (st.project ? p.id === st.project.id : p.path === (st.path ?? j.dest)));
    const main = added?.worktrees.find((w) => w.is_main);
    if (main) selectIfStill(stamp, main.path);
    void refreshSessions();
    showNotice(AP.cloned(added?.name ?? j.name));
    // the project is in: nothing left to do in the dialog
    setShowAddProject(false);
    if (mine === gen) setCloneJob(null);
  } else if (st.state === "cancelled") {
    patch(mine, { state: "cancelled" });
    if (!showAddProject()) showNotice(AP.cloneCancelled);
  } else {
    const why = coded(st.error_code, st.error_params, st.error ?? "");
    patch(mine, { state: "failed", error: why });
    if (!showAddProject()) setError(AP.cloneFailed(why));
  }
};

const poll = async (mine: number, id: string) => {
  let errors = 0;
  while (mine === gen) {
    await sleep(POLL_MS);
    if (mine !== gen) return;
    let st: CloneStatus;
    try {
      st = await projectCloneStatus(id);
      errors = 0;
    } catch (e) {
      if (++errors < MAX_POLL_ERRORS) continue;
      patch(mine, { state: "failed", error: errorText(e) });
      return;
    }
    if (st.state === "running") patch(mine, { state: "running", percent: st.percent, phase: st.phase });
    else {
      await finish(mine, st);
      return;
    }
  }
};

/** Starts `git clone` as a daemon job and follows it until it ends. Does nothing while another clone runs. */
export const startClone = async (p: CloneParams): Promise<void> => {
  if (cloneActive()) return;
  const mine = ++gen;
  cancelAsked = false;
  const name = repoNameFromUrl(p.url) || p.dest.split(/[\\/]/).filter(Boolean).pop() || p.dest;
  setCloneJob({ state: "starting", id: "", dest: p.dest, name });
  let id: string;
  try {
    id = (await projectClone(p)).id;
  } catch (e) {
    patch(mine, { state: "failed", error: errorText(e) });
    return;
  }
  if (mine !== gen) return;
  patch(mine, { state: "running", id });
  if (cancelAsked) void projectCloneCancel(id).catch((e) => console.debug("project.clone_cancel", e));
  await poll(mine, id);
};

/** Stops the clone in flight; the daemon kills git and removes the partial folder, and the next poll reports `cancelled`. */
export const cancelClone = async (): Promise<void> => {
  const j = cloneJob();
  if (!cloneActive(j) || !j) return;
  cancelAsked = true;
  if (!j.id) return;
  try {
    await projectCloneCancel(j.id);
  } catch (e) {
    setError(errorText(e));
  }
};

/** Forgets a finished job (its result was read). */
export const dismissClone = () => {
  if (!cloneActive()) setCloneJob(null);
};

/** Drops the job and stops following it (tests, and a clean start). */
export const resetClone = () => {
  gen++;
  cancelAsked = false;
  setCloneJob(null);
};
