import { describe, expect, it } from "vitest";
import { findPathLinks, parsePathLink, resolvePathLink, safeLinkUrl } from "./pathlinks";

describe("parsePathLink", () => {
  it("splits line and column", () => {
    expect(parsePathLink("src/a.ts:12:3")).toEqual({ path: "src/a.ts", line: 12, col: 3 });
    expect(parsePathLink("src/a.ts:12")).toEqual({ path: "src/a.ts", line: 12, col: undefined });
  });
  it("keeps a Windows drive colon and drops trailing punctuation", () => {
    expect(parsePathLink("C:\\w\\a.rs:7")).toEqual({ path: "C:\\w\\a.rs", line: 7, col: undefined });
    expect(parsePathLink("a.ts.")).toEqual({ path: "a.ts", line: undefined, col: undefined });
  });
});

describe("findPathLinks", () => {
  it("finds relative and absolute paths with their column", () => {
    const l = findPathLinks("error in src/app/main.tsx:42:7 and C:\\proj\\lib.rs:3");
    expect(l.map((x) => [x.path, x.line, x.start])).toEqual([
      ["src/app/main.tsx", 42, 9],
      ["C:\\proj\\lib.rs", 3, 35],
    ]);
  });
  it("ignores paths that are part of a URL", () => {
    expect(findPathLinks("see https://example.com/page.html now")).toEqual([]);
  });
  it("drops trailing punctuation from the link text", () => {
    expect(findPathLinks("(open a.ts), then")[0].text).toBe("a.ts");
  });
  it("finds nothing in plain text", () => {
    expect(findPathLinks("compiling... done")).toEqual([]);
  });
});

describe("resolvePathLink", () => {
  it("makes paths inside the worktree relative", () => {
    expect(resolvePathLink("C:\\w\\proj\\src\\a.ts", "C:/w/proj")).toEqual({ rel: "src/a.ts" });
    expect(resolvePathLink("./src/a.ts", "C:/w/proj")).toEqual({ rel: "src/a.ts" });
  });
  it("is case-insensitive on the drive path and flags outside paths", () => {
    expect(resolvePathLink("c:/W/Proj/x.ts", "C:/w/proj/")).toEqual({ rel: "x.ts" });
    expect(resolvePathLink("D:\\other\\x.ts", "C:/w/proj")).toEqual({ outside: "D:\\other\\x.ts" });
  });
  it("does not treat a sibling folder with the same prefix as inside", () => {
    expect(resolvePathLink("C:/w/proj2/x.ts", "C:/w/proj")).toEqual({ outside: "C:/w/proj2/x.ts" });
  });
});

describe("resolvePathLink escapes", () => {
  it("treats a relative path that climbs out as outside", () => {
    expect(resolvePathLink("../secret.txt", "C:/w/proj")).toEqual({ outside: "../secret.txt" });
    expect(resolvePathLink("src/../../x.ts", "C:/w/proj")).toEqual({ outside: "src/../../x.ts" });
  });
});

describe("safeLinkUrl", () => {
  it("allows only http and https", () => {
    expect(safeLinkUrl("https://example.com/a")).toBe("https://example.com/a");
    expect(safeLinkUrl("http://localhost:3000")).toBe("http://localhost:3000/");
    for (const bad of ["mailto:a@b.c", "tel:123", "file:///C:/x", "javascript:alert(1)", "ssh://h", "not a url", ""])
      expect(safeLinkUrl(bad)).toBeNull();
  });
});
