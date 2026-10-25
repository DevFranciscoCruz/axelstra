import { describe, expect, it } from "vitest";
import { safeLinkUrl } from "../pathlinks";
import { findHtmlLinks } from "./htmlLinks";

const paths = (line: string) => findHtmlLinks(line).map((l) => l.path);
const textOf = (line: string) => findHtmlLinks(line).map((l) => line.slice(l.start, l.end));

describe("findHtmlLinks", () => {
  it("finds Windows drive paths, backslash and slash", () => {
    expect(paths(String.raw`wrote C:\Users\x\index.html ok`)).toEqual([String.raw`C:\Users\x\index.html`]);
    expect(paths("wrote C:/Users/x/page.htm ok")).toEqual(["C:/Users/x/page.htm"]);
  });

  it("keeps spaces in a drive path bounded by the drive and the extension", () => {
    expect(paths(String.raw`Saved C:\Users\x y\My Docs\index.html now`)).toEqual([
      String.raw`C:\Users\x y\My Docs\index.html`,
    ]);
    // two paths on a line stay two paths
    expect(paths(String.raw`C:\a\b.html and C:\c d\e.html`)).toEqual([
      String.raw`C:\a\b.html`,
      String.raw`C:\c d\e.html`,
    ]);
    // the drive path does not swallow the text up to a later separator
    expect(paths(String.raw`C:\a\b.html and then \x\y.html`)).toEqual([String.raw`C:\a\b.html`, String.raw`\x\y.html`]);
  });

  it("finds POSIX absolute and relative paths", () => {
    expect(paths("open /home/u/site/index.html please")).toEqual(["/home/u/site/index.html"]);
    expect(paths("see ./a/b.html")).toEqual(["./a/b.html"]);
    expect(paths("see a/b.html")).toEqual(["a/b.html"]);
    expect(paths(String.raw`see ..\x.htm`)).toEqual([String.raw`..\x.htm`]);
    expect(paths("report.html")).toEqual(["report.html"]);
  });

  it("links spaces only when quoted or drive-bounded", () => {
    expect(paths('open "my site/index page.html" now')).toEqual(["my site/index page.html"]);
    expect(paths("open 'my site/a.html'")).toEqual(["my site/a.html"]);
    expect(paths("open `my site/a.html`")).toEqual(["my site/a.html"]);
    // unquoted POSIX/relative: stops at the space
    expect(paths("open my site/a.html")).toEqual(["site/a.html"]);
  });

  it("takes :line and :line:col without making them part of the path", () => {
    const [a] = findHtmlLinks("error in a/b.html:12");
    expect(a).toMatchObject({ path: "a/b.html", line: 12 });
    expect(a.col).toBeUndefined();
    const [b] = findHtmlLinks(String.raw`at C:\x\y.html:3:14`);
    expect(b).toMatchObject({ path: String.raw`C:\x\y.html`, line: 3, col: 14 });
    expect(textOf("a/b.html:12:5 bad")).toEqual(["a/b.html:12:5"]);
  });

  it("gives the 0-based range of the underlined text, quotes excluded", () => {
    const [a] = findHtmlLinks("ab a.html");
    expect([a.start, a.end]).toEqual([3, 9]);
    const [q] = findHtmlLinks('x "a b.html" y');
    expect([q.start, q.end]).toEqual([3, 11]);
  });

  it("strips trailing punctuation", () => {
    expect(paths("(see a/b.html)")).toEqual(["a/b.html"]);
    expect(paths("[a/b.html]")).toEqual(["a/b.html"]);
    expect(paths("wrote a/b.html.")).toEqual(["a/b.html"]);
    expect(paths("a/b.html, c/d.html;")).toEqual(["a/b.html", "c/d.html"]);
    expect(textOf("a/b.html:7.")).toEqual(["a/b.html:7"]);
    expect(paths(String.raw`(C:\x\y.html).`)).toEqual([String.raw`C:\x\y.html`]);
  });

  it("does not match URLs (the web-links handling owns them)", () => {
    expect(paths("https://x/y.html")).toEqual([]);
    expect(paths("see http://localhost:3000/a/b.htm and more")).toEqual([]);
    expect(paths("file:///C:/x/y.html")).toEqual([]);
    // a URL does not hide a real path on the same line
    expect(paths("https://x/y.html vs ./z.html")).toEqual(["./z.html"]);
  });

  it("does not match other extensions or longer names", () => {
    expect(paths("a/b.htmlx")).toEqual([]);
    expect(paths("a/b.html.bak")).toEqual([]);
    expect(paths("a/b.html-old")).toEqual([]);
    expect(paths("a/b.xhtml")).toEqual([]);
    expect(paths("a/b.txt c.js .html")).toEqual([]);
    expect(paths("a.html/b")).toEqual([]);
  });

  it("is case-insensitive on the extension", () => {
    expect(paths("A/B.HTML")).toEqual(["A/B.HTML"]);
  });
});

describe("existing http(s) link handling", () => {
  it("still accepts an https url that ends in .html, and refuses non-http schemes", () => {
    expect(safeLinkUrl("https://example.com/a/b.html")).toBe("https://example.com/a/b.html");
    expect(safeLinkUrl("file:///C:/x/y.html")).toBeNull();
  });
});
