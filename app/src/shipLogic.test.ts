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

import { applySuggestion, canCreatePr, canGenerate, genInitial, genReduce, generateLabel } from "./shipLogic";

describe("generate button state", () => {
  it("goes idle -> loading -> done and ignores a second click while loading", () => {
    let s = genReduce(genInitial, { kind: "start" });
    expect(s).toEqual({ phase: "loading", seq: 1 });
    expect(genReduce(s, { kind: "start" })).toBe(s);
    expect(canGenerate(s, true)).toBe(false);
    s = genReduce(s, { kind: "ok", seq: 1 });
    expect(s.phase).toBe("done");
    expect(canGenerate(s, true)).toBe(true);
    expect(canGenerate(s, false)).toBe(false);
    expect(generateLabel(genInitial)).toBe("Gerar título e descrição");
    expect(generateLabel({ phase: "loading", seq: 1 })).toBe("A gerar…");
    expect(generateLabel(s)).toBe("Gerar de novo");
  });
  it("can fail and be retried; a late answer of an older request is dropped", () => {
    let s = genReduce(genInitial, { kind: "start" });
    s = genReduce(s, { kind: "fail", seq: 1 });
    expect(s.phase).toBe("error");
    expect(canGenerate(s, true)).toBe(true);
    s = genReduce(s, { kind: "start" });
    expect(s.seq).toBe(2);
    expect(genReduce(s, { kind: "ok", seq: 1 })).toBe(s);
    s = genReduce(s, { kind: "reset" });
    expect(s).toEqual({ phase: "idle", seq: 2 });
    const next = genReduce(s, { kind: "start" });
    expect(genReduce(next, { kind: "ok", seq: 2 })).toBe(next);
  });
  it("applies a suggestion with a title, trimmed, and keeps the fields otherwise", () => {
    const cur = { title: "mine", body: "b" };
    expect(applySuggestion(cur, { title: "  feat: x ", body: "\nbody\n" })).toEqual({ title: "feat: x", body: "body" });
    expect(applySuggestion(cur, { title: "  ", body: "x" })).toBe(cur);
  });
  it("creates only with a title and when idle", () => {
    expect(canCreatePr(" ", false)).toBe(false);
    expect(canCreatePr("t", true)).toBe(false);
    expect(canCreatePr("t", false)).toBe(true);
  });
});

describe("problemText with a message code", () => {
  it("shows the daemon's own detail in the UI language and gh's words as they are", () => {
    const coded = { problem: "error" as const, detail: "gh took too long", detail_code: "gh.timeout", detail_params: { cmd: "pr view", secs: "15" } };
    expect(problemText(coded).text).toContain("pr view");
    expect(problemText(coded).text).not.toBe("gh took too long");
    expect(problemText({ problem: "error", detail: "HTTP 502" }).text).toBe("HTTP 502");
    expect(problemText({ problem: "error", detail: "x", detail_code: "gh.unknown_to_this_app" }).text).toBe("x");
  });
});
