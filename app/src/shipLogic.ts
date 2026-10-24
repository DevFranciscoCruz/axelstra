import type { CheckItem, ShipStatus } from "./api";
import { P } from "./strings.panels";
import { lazy, lazyPair } from "./i18n";

/** how often the PR state is read while the panel is on screen and the window has focus */
export const SHIP_POLL_MS = 20_000;

/** [icon, word]: the word is the cue that does not depend on colour */
export const CI_LABEL: Record<ShipStatus["ci"], [string, string]> = {
  pass: lazyPair("check", () => P.ship.ci.pass),
  fail: lazyPair("x", () => P.ship.ci.fail),
  pending: lazyPair("ellipsis", () => P.ship.ci.pending),
  none: lazyPair("", () => P.ship.ci.none),
};
export const CHECK_LABEL: Record<CheckItem["state"], [string, string]> = {
  pass: lazyPair("check", () => P.ship.check.pass),
  fail: lazyPair("x", () => P.ship.check.fail),
  pending: lazyPair("ellipsis", () => P.ship.check.pending),
  skipped: lazyPair("", () => P.ship.check.skipped),
};
export const REVIEW_LABEL: Record<ShipStatus["review"], string> = lazy({
  approved: () => P.ship.review.approved,
  changes: () => P.ship.review.changes,
  review: () => P.ship.review.review,
  "": () => P.ship.review.none,
});

/** one phrase for "can this be merged": what blocks it, or that nothing does */
export const mergeLabel = (s: ShipStatus): string => {
  if (s.pr_state === "merged") return P.ship.merged;
  if (s.pr_state === "closed") return P.ship.closed;
  if (s.draft) return P.ship.draft;
  if (s.mergeable === "no" || s.merge_state === "dirty") return P.ship.conflicts;
  if (s.can_merge) return P.ship.ready;
  if (s.merge_state === "behind") return P.ship.behind;
  if (s.ci === "fail") return P.ship.blockedChecks;
  if (s.ci === "pending") return P.ship.waitingChecks;
  if (s.merge_state === "blocked") return P.ship.blocked;
  return P.ship.computing;
};

/** auto-merge makes sense while the PR is open and not yet mergeable: GitHub merges it when the checks pass */
export const canAutoMerge = (s: ShipStatus): boolean =>
  s.pr_state === "open" && !s.draft && !s.can_merge && s.mergeable !== "no";

export const problemText = (s: Pick<ShipStatus, "problem" | "detail">): { text: string; fix: string } => {
  switch (s.problem) {
    case "missing":
      return P.ship.missing;
    case "auth":
      return P.ship.auth;
    case "nopr":
      return P.ship.nopr;
    default:
      return { text: s.detail || P.ship.unreadable.text, fix: P.ship.unreadable.fix };
  }
};

/** the window must be showing the panel and have focus, otherwise a poll is wasted */
export const shouldPoll = (hidden: boolean, focused: boolean) => !hidden && focused;

export const mergeQuestion = (s: Pick<ShipStatus, "number" | "title">, branch: string, deleteBranch: boolean) =>
  P.ship.mergeAsk(s.number, s.title, branch, deleteBranch);

export const autoMergeQuestion = (s: Pick<ShipStatus, "number" | "title">) => P.ship.autoAsk(s.number, s.title);

/** The "Gerar título e descrição" button: idle -> loading -> done | error. `seq` numbers the requests so that the answer of
 *  an older one (the user switched worktree, or asked again) is dropped. */
export type GenState = { phase: "idle" | "loading" | "done" | "error"; seq: number };
export type GenEvent =
  { kind: "start" } | { kind: "ok"; seq: number } | { kind: "fail"; seq: number } | { kind: "reset" };

export const genInitial: GenState = { phase: "idle", seq: 0 };

export const genReduce = (s: GenState, e: GenEvent): GenState => {
  switch (e.kind) {
    case "start":
      // a second click while one is running does nothing
      return s.phase === "loading" ? s : { phase: "loading", seq: s.seq + 1 };
    case "ok":
      return s.phase === "loading" && e.seq === s.seq ? { ...s, phase: "done" } : s;
    case "fail":
      return s.phase === "loading" && e.seq === s.seq ? { ...s, phase: "error" } : s;
    case "reset":
      // keeps counting, so a late answer of the request that was running cannot match a later one
      return { phase: "idle", seq: s.seq };
  }
};

export const canGenerate = (s: GenState, hasWorktree: boolean): boolean => hasWorktree && s.phase !== "loading";

export const generateLabel = (s: GenState): string =>
  s.phase === "loading"
    ? P.ship.newPr.generating
    : s.phase === "done"
      ? P.ship.newPr.regenerate
      : P.ship.newPr.generate;

/** The answer replaces the fields only when it has a title. */
export const applySuggestion = (
  cur: { title: string; body: string },
  sug: { title: string; body: string },
): { title: string; body: string } => (sug.title.trim() ? { title: sug.title.trim(), body: sug.body.trim() } : cur);

export const canCreatePr = (title: string, busy: boolean): boolean => !busy && title.trim().length > 0;
