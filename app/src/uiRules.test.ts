import { describe, expect, it } from "vitest";

// The look of the app is built from the tokens in :root (see styles.css). These rules keep it that way.
const sheets = import.meta.glob<string>("./*.css", { query: "?raw", import: "default", eager: true });
// vitest hands css modules over empty, so the stylesheets are read from disk like a11yRules.test.ts does
// @ts-expect-error the app has no node typings; the test runs under node
import { readFileSync, readdirSync } from "node:fs";
const files: string[] = readdirSync("src").filter((f: string) => f.endsWith(".css"));
void sheets;

/** the stylesheet without the blocks that define tokens (:root and :root[data-theme=...]) */
const body = (f: string) => (readFileSync(`src/${f}`, "utf8") as string).replace(/:root[^{]*\{[^}]*\}/g, "");
const hits = (re: RegExp) => files.flatMap((f) => body(f).split("\n").map((l, i) => ({ f, i: i + 1, l })).filter((x) => re.test(x.l)).map((x) => `${x.f}:${x.i}: ${x.l.trim().slice(0, 90)}`));

describe("ui tokens", () => {
  it("colours come from variables: no hex outside :root", () => {
    expect(hits(/#[0-9a-fA-F]{6}\b|#[0-9a-fA-F]{3}\b(?![0-9a-fA-F])/)).toEqual([]);
  });
  it("font sizes come from the --fs-* scale", () => {
    expect(hits(/font-size:\s*[0-9.]+px/)).toEqual([]);
  });
  it("corner radii come from the --r-* scale", () => {
    expect(hits(/border-radius:\s*[0-9]+px/)).toEqual([]);
  });
  it("dropdowns use the app Select, not the system <select>", () => {
    const src = import.meta.glob<string>("./*.tsx", { query: "?raw", import: "default", eager: true });
    const bad = Object.entries(src).filter(([, t]) => /<select[\s>]/.test(t)).map(([f]) => f);
    expect(bad).toEqual([]);
  });
});
