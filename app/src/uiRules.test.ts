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
const hits = (re: RegExp) =>
  files.flatMap((f) =>
    body(f)
      .split("\n")
      .map((l, i) => ({ f, i: i + 1, l }))
      .filter((x) => re.test(x.l))
      .map((x) => `${x.f}:${x.i}: ${x.l.trim().slice(0, 90)}`),
  );

describe("ui tokens", () => {
  it("colours come from variables: no hex outside :root", () => {
    expect(hits(/#[0-9a-fA-F]{6}\b|#[0-9a-fA-F]{3}\b(?![0-9a-fA-F])/)).toEqual([]);
  });
  it("rgba colours come from a --*-rgb token, except the black and white of shadows and scrims", () => {
    expect(hits(/rgba?\(\s*(?!var\(|0\s*,\s*0\s*,\s*0(?![\w-])|255\s*,\s*255\s*,\s*255(?![\w-]))\d/)).toEqual([]);
  });
  it("hover-only actions keep a resting opacity", () => {
    expect(hits(/\.(dk-acts|x|cbtn)(?![\w-])[^{]*\{[^}]*(opacity:\s*0(?![\w-])|visibility:\s*hidden)/)).toEqual([]);
  });
  it("no dead max-width:620px media query", () => {
    expect(hits(/max-width:\s*620px/)).toEqual([]);
  });
  it("Usage and LimitsBar draw with tokens, no literal colours in their markup", () => {
    const src = import.meta.glob<string>(["./Usage.tsx", "./LimitsBar.tsx"], {
      query: "?raw",
      import: "default",
      eager: true,
    });
    const bad = Object.entries(src)
      .filter(([, t]) => /#[0-9a-fA-F]{3,6}(?![0-9a-fA-F])|rgba?\(/.test(t))
      .map(([f]) => f);
    expect(bad).toEqual([]);
  });
  it("font sizes come from the --fs-* scale", () => {
    expect(hits(/font-size:\s*[0-9.]+px/)).toEqual([]);
  });
  it("corner radii come from the --r-* scale", () => {
    expect(hits(/border-radius:\s*[0-9]+px/)).toEqual([]);
  });
  it("dropdowns use the app Select, not the system <select>", () => {
    const src = import.meta.glob<string>("./*.tsx", { query: "?raw", import: "default", eager: true });
    const bad = Object.entries(src)
      .filter(([, t]) => /<select[\s>]/.test(t))
      .map(([f]) => f);
    expect(bad).toEqual([]);
  });
});

describe("design system rules", () => {
  it("box-shadow comes from --shadow-sm/md/lg, never a literal black", () => {
    expect(hits(/box-shadow:[^;]*rgba\(\s*0\s*,\s*0\s*,\s*0/)).toEqual([]);
  });
  it("z-index above 2 comes from the --z-* scale (1 and 2 stack inside a component)", () => {
    expect(hits(/z-index:\s*(?!var\()-?([3-9]|\d\d)/)).toEqual([]);
  });
  it("font-family comes from a token or inherits", () => {
    expect(hits(/font-family:\s*(?!\s|var\(|inherit)/)).toEqual([]);
  });
  it("the shadow and z-index scales exist in both themes' base", () => {
    const root = (readFileSync("src/styles.css", "utf8") as string).match(/:root \{[^}]*\}/)![0];
    for (const t of ["--shadow-sm", "--shadow-md", "--shadow-lg", "--z-pop", "--z-top", "--font-mono"])
      expect(root).toContain(t + ":");
    const light = (readFileSync("src/styles.css", "utf8") as string).match(
      /:root\[data-theme="light"\]\s*\{[^}]*\}/,
    )![0];
    for (const t of ["--shadow-sm", "--shadow-md", "--shadow-lg"]) expect(light).toContain(t + ":");
  });
  it("icons are drawn by Icon.tsx: no <svg outside it (StateDot is the one exception)", () => {
    const src = import.meta.glob<string>(["./**/*.tsx", "!./**/*.test.tsx"], {
      query: "?raw",
      import: "default",
      eager: true,
    });
    const bad = Object.entries(src)
      .filter(([f, t]) => !/\/(Icon|StateDot)\.tsx$/.test(f) && /<svg[\s>]/.test(t))
      .map(([f]) => f);
    expect(bad).toEqual([]);
  });
});
