import { describe, expect, it } from "vitest";
import { formatError, installErrorHooks, type ErrorReport } from "./errorHooks";

describe("formatError", () => {
  it("formats an Error with name, message and stack", () => {
    const e = new TypeError("boom");
    const r = formatError("error", e);
    expect(r.kind).toBe("error");
    expect(r.message).toBe("TypeError: boom");
    expect(r.stack).toContain("boom");
  });

  it("handles strings, objects and undefined", () => {
    expect(formatError("x", "plain").message).toBe("plain");
    expect(formatError("x", { a: 1 }).message).toBe('{"a":1}');
    expect(formatError("x", undefined).message).toBe("undefined");
  });

  it("survives circular objects", () => {
    const o: Record<string, unknown> = {};
    o.self = o;
    expect(formatError("x", o).message).toBe("[object Object]");
  });

  it("appends the location and clips long text", () => {
    const r = formatError("error", "m".repeat(5000), "app.js:1:2");
    expect(r.message.length).toBe(2000);
    expect(r.stack).toBe("at app.js:1:2");
    expect(formatError("e", new Error("x".repeat(10)), "f:1:1").stack).toContain("at f:1:1");
    const long = new Error("e");
    long.stack = "s".repeat(20000);
    expect(formatError("e", long).stack.length).toBe(8000);
  });
});

describe("installErrorHooks", () => {
  it("forwards error and unhandledrejection events, even if send throws", () => {
    const target = new EventTarget();
    const got: ErrorReport[] = [];
    installErrorHooks(
      (r) => {
        got.push(r);
        throw new Error("send failed");
      },
      target as unknown as Window,
    );
    const err = Object.assign(new Event("error"), {
      error: new Error("oops"),
      message: "oops",
      filename: "a.js",
      lineno: 3,
      colno: 4,
    });
    target.dispatchEvent(err);
    target.dispatchEvent(Object.assign(new Event("unhandledrejection"), { reason: "nope" }));
    expect(got).toHaveLength(2);
    expect(got[0].message).toBe("Error: oops");
    expect(got[0].stack).toContain("a.js:3:4");
    expect(got[1]).toMatchObject({ kind: "unhandledrejection", message: "nope" });
  });
});

describe("installErrorHooks onRejection", () => {
  it("hands unhandled rejections to onRejection, not plain errors, and survives it throwing", () => {
    const target = new EventTarget();
    const seen: unknown[] = [];
    installErrorHooks(
      () => {},
      target as unknown as Window,
      (reason) => {
        seen.push(reason);
        throw new Error("toast failed");
      },
    );
    target.dispatchEvent(Object.assign(new Event("error"), { error: new Error("oops"), message: "oops" }));
    target.dispatchEvent(Object.assign(new Event("unhandledrejection"), { reason: "nope" }));
    expect(seen).toEqual(["nope"]);
  });
});
