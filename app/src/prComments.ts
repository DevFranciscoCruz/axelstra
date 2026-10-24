import type { GhItem, ReviewComment, ReviewSummary, Worktree } from "./api";
import { P } from "./strings.panels";

const C = P.pulls.comments;

/** "src/a.rs:42", "src/a.rs" when the line is unknown, "Geral" for a comment on the PR as a whole */
export const locationLabel = (c: Pick<ReviewComment, "path" | "line">): string =>
  !c.path ? C.noLocation : c.line > 0 ? `${c.path}:${c.line}` : c.path;

/** The text as shown in the list: blank lines collapsed, cut at `max` characters (by code point) with an ellipsis. */
export const displayBody = (body: string, max = 600): string => {
  const t = body
    .replace(/\r\n?/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  const chars = Array.from(t);
  return chars.length > max ? `${chars.slice(0, max).join("").trimEnd()}…` : t;
};

/** short hints shown next to a comment: outdated code, a reply */
export const commentHints = (c: Pick<ReviewComment, "outdated" | "in_reply_to">): string[] => [
  ...(c.outdated ? [C.outdated] : []),
  ...(c.in_reply_to ? [C.reply] : []),
];

export type Thread = { root: ReviewComment; replies: ReviewComment[] };

/** Comments grouped by thread, in the order of their roots. A reply whose root is not in the list (outside the pages
 *  that were read) is a thread of its own. */
export const threads = (list: ReviewComment[]): Thread[] => {
  const byId = new Map(list.map((c) => [c.id, c]));
  const rootOf = (c: ReviewComment): ReviewComment => {
    let cur = c;
    for (let i = 0; i < 50 && cur.in_reply_to && byId.has(cur.in_reply_to); i++) cur = byId.get(cur.in_reply_to)!;
    return cur;
  };
  const out = new Map<number, Thread>();
  for (const c of list) {
    const root = rootOf(c);
    const t = out.get(root.id) ?? { root, replies: [] };
    if (c.id !== root.id) t.replies.push(c);
    out.set(root.id, t);
  }
  return [...out.values()];
};

/** a review summary as one line: "bob pediu alterações" */
export const reviewLine = (r: Pick<ReviewSummary, "author" | "state">): string =>
  `${r.author} ${C.reviewState[r.state] ?? C.reviewState.commented}`;

/** The daemon says this when nobody runs as the worktree's main agent. */
export const isNoLeadError = (e: unknown): boolean =>
  /no live lead/i.test(String((e as { message?: string })?.message ?? e));

/** The worktree whose branch is the PR's head branch, if any: comments can only be sent to an agent that works there. */
export const worktreeForPr = (
  item: Pick<GhItem, "branch">,
  worktrees: Pick<Worktree, "path" | "branch">[],
): string | null => (item.branch ? (worktrees.find((w) => w.branch === item.branch)?.path ?? null) : null);

/** What the confirm step shows: the text exactly as the daemon will deliver it. */
export type SendStep =
  | { step: "idle" }
  | { step: "loading"; comment: number }
  | { step: "confirm"; comment: number; text: string }
  | { step: "sending"; comment: number; text: string };

/** Enter sends and Esc cancels, but only while the confirm step is showing and nothing is being sent. */
export const confirmKey = (s: SendStep, key: string): "send" | "cancel" | null => {
  if (s.step !== "confirm") return null;
  return key === "Enter" ? "send" : key === "Escape" ? "cancel" : null;
};
