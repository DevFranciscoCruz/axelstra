import { describe, expect, it } from "vitest";
import { decideWait } from "./waitNotice";
import type { WaitInfo } from "./api";

const free = { grace: false, watching: false };
const perm: WaitInfo = { kind: "permission", text: "Run rm -rf build?" };

describe("decideWait", () => {
  it("a new wait notifies once with the agent's words", () => {
    const d = decideWait(undefined, perm, "main", "wt", free);
    expect(d.text).toContain("Run rm -rf build?");
    expect(d.text).toContain("wt");
    expect(d.os).toBe(true);
  });

  it("the same wait again does not notify", () => {
    const first = decideWait(undefined, perm, "main", "wt", free);
    const again = decideWait(first.seen, { ...perm }, "main", "wt", free);
    expect(again.text).toBeNull();
    expect(again.seen).toBe(first.seen);
  });

  it("a cleared wait resets, a new one notifies again", () => {
    const first = decideWait(undefined, perm, "main", "wt", free);
    const cleared = decideWait(first.seen, null, "main", "wt", free);
    expect(cleared.seen).toBeUndefined();
    expect(decideWait(cleared.seen, perm, "main", "wt", free).text).not.toBeNull();
  });

  it("a changed text or kind is a new wait", () => {
    const first = decideWait(undefined, perm, "main", "wt", free);
    expect(decideWait(first.seen, { ...perm, text: "other" }, "main", "wt", free).text).not.toBeNull();
    expect(decideWait(first.seen, { kind: "question", text: perm.text }, "main", "wt", free).text).not.toBeNull();
  });

  it("idle_input never notifies", () => {
    const d = decideWait(undefined, { kind: "idle_input", text: "ready" }, "main", "wt", free);
    expect(d).toEqual({ seen: undefined, text: null, os: false });
  });

  it("empty text uses the generic line of the kind", () => {
    const p = decideWait(undefined, { kind: "permission", text: "  " }, "main", "wt", free);
    const q = decideWait(undefined, { kind: "question", text: "" }, "main", "wt", free);
    expect(p.text).toBeTruthy();
    expect(q.text).toBeTruthy();
    expect(p.text).not.toBe(q.text);
    expect(p.text).not.toMatch(/: $/);
  });

  it("watching keeps the bell entry but skips the OS notification", () => {
    const d = decideWait(undefined, perm, "main", "wt", { grace: false, watching: true });
    expect(d.text).not.toBeNull();
    expect(d.os).toBe(false);
  });

  it("during the start grace nothing is announced and the wait is remembered", () => {
    const d = decideWait(undefined, perm, "main", "wt", { grace: true, watching: false });
    expect(d.text).toBeNull();
    expect(d.seen).toBeDefined();
    expect(decideWait(d.seen, perm, "main", "wt", free).text).toBeNull();
  });
});
