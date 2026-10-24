import { describe, expect, it } from "vitest";
import { isBypass, planStart, restorePayload } from "./kanbanLogic";
import { needsTypedName, removeRisks } from "./removeRisk";
import { typedMatches } from "./Confirm";

const card = (id: string, mode: string, bypass = false, acceptance: string | null = null) => ({ id, agent: "claude", permission_mode: mode, bypass, acceptance });

describe("planStart", () => {
  it("shows each card's mode, flags bypass and sends the pin", () => {
    const p = planStart(2, { pin: "abc", cards: [card("a", "default"), card("b", "bypassPermissions", true, "make test")] }, []);
    expect(p.message).toContain("⚠ b: claude (bypassPermissions)");
    expect(p.message).toContain("a: claude (default)");
    expect(p.bypass).toEqual(["b"]);
    expect(p.pin).toBe("abc");
    expect(p.trustAcceptance).toBe(true);
    expect(p.warn).toBe(true);
  });
  it("detects bypass by mode name even when the flag is missing", () => {
    expect(isBypass("bypass-permissions")).toBe(true);
    expect(isBypass("acceptEdits")).toBe(false);
    expect(planStart(1, { pin: "p", cards: [card("a", "bypassPermissions")] }, []).bypass).toEqual(["a"]);
  });
  it("plain cards: no warning, no pin without trust info", () => {
    expect(planStart(1, { pin: "p", cards: [card("a", "default")] }, []).warn).toBe(false);
    const legacy = planStart(1, null, ["a: x"]);
    expect(legacy.pin).toBeUndefined();
    expect(legacy.trustAcceptance).toBe(true);
  });
});

describe("restorePayload", () => {
  it("keeps the id and every field", () => {
    const p = restorePayload({ meta: { id: "k1", title: "T", status: "todo", agent: "claude", depends_on: ["k0"], priority: 2, acceptance: "ok" }, body: "b" });
    expect(p).toEqual({ id: "k1", title: "T", body: "b", status: "todo", agent: "claude", priority: 2, depends_on: ["k0"], acceptance: "ok" });
  });
});

describe("remove worktree confirm", () => {
  it("types the name only when work would be lost", () => {
    expect(needsTypedName({ dirty: 2, ahead: 0, upstream: true })).toBe(true);
    expect(needsTypedName({ dirty: 0, ahead: 1, upstream: false })).toBe(true);
    expect(needsTypedName({ dirty: 0, ahead: 0, upstream: true })).toBe(false);
    expect(needsTypedName(undefined)).toBe(false);
  });
  it("describes the risks", () => {
    expect(removeRisks({ dirty: 1, ahead: 2, upstream: false }, 1)).toHaveLength(3);
  });
  it("typedMatches ignores case and spaces but never an empty target", () => {
    expect(typedMatches("Foo", " foo ")).toBe(true);
    expect(typedMatches("Foo", "fo")).toBe(false);
    expect(typedMatches("", "")).toBe(false);
  });
});
