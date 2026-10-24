import { describe, expect, it } from "vitest";
import type { ReviewComment } from "./api";
import {
  commentHints,
  confirmKey,
  displayBody,
  isNoLeadError,
  locationLabel,
  reviewLine,
  threads,
  worktreeForPr,
} from "./prComments";

const c = (id: number, over: Partial<ReviewComment> = {}): ReviewComment => ({
  id,
  path: "src/a.rs",
  line: 3,
  body: "b",
  author: "bob",
  diff_hunk: "",
  created: "",
  url: "",
  in_reply_to: 0,
  outdated: false,
  pr: 7,
  ...over,
});

describe("locationLabel", () => {
  it("shows file:line, the file alone, or a general label", () => {
    expect(locationLabel(c(1))).toBe("src/a.rs:3");
    expect(locationLabel(c(1, { line: 0 }))).toBe("src/a.rs");
    expect(locationLabel(c(1, { path: "", line: 0 }))).toBe("Geral");
  });
});

describe("displayBody", () => {
  it("collapses blank lines and cuts long text on a code point", () => {
    expect(displayBody("a\r\n\r\n\r\n\r\nb\n")).toBe("a\n\nb");
    expect(displayBody("x".repeat(10), 4)).toBe("xxxx…");
    expect(displayBody("😀".repeat(5), 2)).toBe("😀😀…");
    expect(displayBody("short", 10)).toBe("short");
  });
});

describe("hints and threads", () => {
  it("hints outdated code and replies", () => {
    expect(commentHints(c(1))).toEqual([]);
    expect(commentHints(c(1, { outdated: true, in_reply_to: 4 }))).toEqual(["desatualizado", "resposta"]);
  });
  it("groups replies under their root, also through a chain, and keeps orphans apart", () => {
    const list = [c(1), c(2, { in_reply_to: 1 }), c(3), c(4, { in_reply_to: 2 }), c(9, { in_reply_to: 99 })];
    const t = threads(list);
    expect(t.map((x) => [x.root.id, x.replies.map((r) => r.id)])).toEqual([
      [1, [2, 4]],
      [3, []],
      [9, []],
    ]);
  });
  it("does not loop on a reply cycle", () => {
    expect(threads([c(1, { in_reply_to: 2 }), c(2, { in_reply_to: 1 })]).length).toBeGreaterThan(0);
  });
  it("words a review", () => {
    expect(reviewLine({ author: "bob", state: "changes" })).toBe("bob pediu alterações");
  });
});

describe("errors, worktrees and keys", () => {
  it("recognises the missing lead from the daemon's error", () => {
    expect(isNoLeadError(new Error("no live lead in this worktree: start its main agent first"))).toBe(true);
    expect(isNoLeadError("gh api failed: 404")).toBe(false);
  });
  it("finds the worktree of a PR by branch", () => {
    const wts = [
      { path: "/r", branch: "main" },
      { path: "/r/wt", branch: "ax/fix" },
    ];
    expect(worktreeForPr({ branch: "ax/fix" }, wts)).toBe("/r/wt");
    expect(worktreeForPr({ branch: "other" }, wts)).toBeNull();
    expect(worktreeForPr({ branch: "" }, wts)).toBeNull();
  });
  it("Enter sends and Esc cancels only in the confirm step", () => {
    const s = { step: "confirm", comment: 1, text: "t" } as const;
    expect(confirmKey(s, "Enter")).toBe("send");
    expect(confirmKey(s, "Escape")).toBe("cancel");
    expect(confirmKey(s, "a")).toBeNull();
    expect(confirmKey({ step: "sending", comment: 1, text: "t" }, "Enter")).toBeNull();
    expect(confirmKey({ step: "idle" }, "Escape")).toBeNull();
  });
});
