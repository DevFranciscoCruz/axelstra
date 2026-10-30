import { type Widen, count } from "./i18n";
import type { ptD } from "./strings.disk";

/** English texts of disk hygiene (worktree sizes, "Clean builds", the unintegrated-work warning, Settings, notices). */
export const enD: Widen<typeof ptD> = {
  panel: {
    head: "Disk",
    volume: (root: string, free: string, total: string) => `${root} ${free} free of ${total}`,
    low: "low space",
    loading: "Measuring…",
    empty: "No worktree size measured",
    measure: "Measure again",
    active: "in use",
    sizeTip: (target: string, nm: string, cache: string, reclaim: string) =>
      `target: ${target}\nnode_modules: ${nm}\ncaches: ${cache}\nReclaimable: ${reclaim}`,
    clean: "Clean builds",
    cleanTip:
      "Deletes this worktree's target, node_modules and caches (they can be rebuilt); the worktree and its work stay",
    cleanAll: "Clean builds of inactive ones",
    cleanAllTip: "Deletes the builds of every worktree with no session or build running",
    cleaning: "Cleaning…",
  },
  ask: {
    title: "Clean builds",
    ok: "Clean",
    one: (name: string, size: string) =>
      `Delete the builds of ${name} (target, node_modules, caches) and free ${size}? The worktree and its code stay; the next build rebuilds them.`,
    all: (n: number, size: string) =>
      `Delete the builds of ${count(n, "inactive worktree", "inactive worktrees")} and free ${size}? The worktrees and their code stay; the next build rebuilds them.`,
  },
  result: {
    nothing: "Nothing to free",
    freed: (size: string) => `Builds deleted: ${size} freed`,
    failed: (why: string) => `Could not clean the builds: ${why}`,
  },
  stale: {
    head: "Unintegrated work left idle",
    line: (branch: string, commits: number, into: string, ago: string, size: string) =>
      `${branch}: ${count(commits, "commit", "commits")} not in ${into}, idle ${ago}, ${size}`,
    integrate: "Integrate",
    open: "Open",
  },
  notify: {
    kind: "Disk",
    artifacts: (branch: string, size: string) => `Builds of ${branch} deleted: ${size} freed`,
    artifactsAgg: (n: number, size: string) =>
      `Builds of ${count(n, "inactive worktree", "inactive worktrees")} deleted: ${size} freed`,
    pending: (branch: string, holder: string) =>
      holder
        ? `Worktree ${branch} was not removed: its folder is held by ${holder}`
        : `Worktree ${branch} was not removed: its folder is held by another process`,
  },
  settings: {
    title: "Disk and builds",
    note: "The builds (target, node_modules, caches) of idle worktrees are deleted by themselves; the worktree and its work never are. When space is low, the oldest and biggest go first.",
    enabled: "Watch the free disk space",
    minFreeGb: "Low space below (GB)",
    minFreePct: "or below (% of the disk)",
    marginGb: "Free up to this much more (GB)",
    quietHours: "Delete the builds of a worktree idle for (hours)",
  },
};
