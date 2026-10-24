import { describe, expect, it } from "vitest";
import type { ShipStatus } from "./api";
import { autoMergeQuestion, canAutoMerge, mergeLabel, mergeQuestion, problemText, shouldPoll } from "./shipLogic";

const st = (over: Partial<ShipStatus> = {}): ShipStatus => ({
  problem: "", detail: "", number: 7, title: "Fix it", url: "u", pr_state: "open", draft: false, ci: "pass", checks: [], review: "", mergeable: "yes", merge_state: "clean", can_merge: true, ...over,
});

describe("mergeLabel", () => {
  it("names what blocks the merge", () => {
    expect(mergeLabel(st())).toBe("Pronto para merge");
    expect(mergeLabel(st({ pr_state: "merged" }))).toBe("Merge feito");
    expect(mergeLabel(st({ pr_state: "closed" }))).toBe("Fechado sem merge");
    expect(mergeLabel(st({ draft: true, can_merge: false }))).toBe("Rascunho");
    expect(mergeLabel(st({ mergeable: "no", merge_state: "dirty", can_merge: false }))).toBe("Com conflitos");
    expect(mergeLabel(st({ ci: "fail", can_merge: false }))).toBe("Bloqueado pelos checks");
    expect(mergeLabel(st({ ci: "pending", can_merge: false }))).toBe("À espera dos checks");
    expect(mergeLabel(st({ merge_state: "behind", can_merge: false }))).toBe("Branch desatualizado");
  });
});

describe("canAutoMerge", () => {
  it("is offered while open, not draft, not yet mergeable, without conflicts", () => {
    expect(canAutoMerge(st({ ci: "pending", can_merge: false }))).toBe(true);
    expect(canAutoMerge(st())).toBe(false);
    expect(canAutoMerge(st({ draft: true, can_merge: false }))).toBe(false);
    expect(canAutoMerge(st({ mergeable: "no", can_merge: false }))).toBe(false);
    expect(canAutoMerge(st({ pr_state: "merged", can_merge: false }))).toBe(false);
  });
});

describe("problemText", () => {
  it("tells how to fix gh problems in pt-PT", () => {
    expect(problemText({ problem: "auth", detail: "" }).fix).toContain("gh auth login");
    expect(problemText({ problem: "missing", detail: "" }).fix).toContain("gh auth login");
    expect(problemText({ problem: "nopr", detail: "" }).text).toContain("PR");
    expect(problemText({ problem: "error", detail: "HTTP 502" }).text).toBe("HTTP 502");
  });
});

describe("shouldPoll", () => {
  it("needs a visible and focused window", () => {
    expect(shouldPoll(false, true)).toBe(true);
    expect(shouldPoll(true, true)).toBe(false);
    expect(shouldPoll(false, false)).toBe(false);
  });
});

describe("questions", () => {
  it("carry the PR title and whether the branch goes", () => {
    expect(mergeQuestion(st(), "feat", false)).toContain("Fix it");
    expect(mergeQuestion(st(), "feat", false)).not.toContain("apagado");
    expect(mergeQuestion(st(), "feat", true)).toContain("apagado");
    expect(autoMergeQuestion(st())).toContain("#7");
  });
});
