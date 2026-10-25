import { describe, expect, it } from "vitest";
// @ts-expect-error the app has no node typings; the test runs under node
import { readFileSync } from "node:fs";
import { contrast } from "./a11yRules";

// vitest runs from app/; the sources are read from disk
const read = (f: string): string => readFileSync(f, "utf8");
const select = read("src/ui/select.tsx");
const css = read("src/styles.css");
const tw = read("src/tailwind.css");
const AGENTS = ["claude", "codex", "gemini"];

describe("agent logo colours under the CSP", () => {
  it("the Select has no style= (the CSP forbids it) and no colour table in code", () => {
    expect(select).not.toMatch(/\sstyle=/);
    expect(select).not.toMatch(/\bBRAND\b/);
    expect(select).toContain("data-agent=");
  });
  it("every agent has a rule on data-agent and a token per theme with 3:1 on the panel", () => {
    const dark = css.slice(0, css.indexOf(':root[data-theme="light"]'));
    const light = css.slice(css.indexOf(':root[data-theme="light"]'));
    const lightBlock = light.slice(0, light.indexOf("}"));
    for (const a of AGENTS) {
      expect(tw).toContain(`[data-agent="${a}"] { color: var(--brand-${a}); }`);
      const re = new RegExp(String.raw`--brand-${a}:\s*(#[0-9a-f]{6})`, "i");
      const d = re.exec(dark)?.[1];
      const l = re.exec(lightBlock)?.[1];
      expect(d, `dark ${a}`).toBeTruthy();
      expect(l, `light ${a}`).toBeTruthy();
      expect(contrast(d!, "#24262c")).toBeGreaterThanOrEqual(3);
      expect(contrast(l!, "#ffffff")).toBeGreaterThanOrEqual(3);
    }
  });
});
