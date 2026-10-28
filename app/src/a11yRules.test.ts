import { describe, expect, it } from "vitest";
// @ts-expect-error the app has no node typings; the test runs under node
import { readFileSync } from "node:fs";
import { contrast, openTags } from "./a11yRules";

// vitest hands css modules over empty, so the stylesheet is read from disk
const css: string = readFileSync("src/styles.css", "utf8"); // vitest runs from app/
const sources = import.meta.glob<string>("./*.tsx", { query: "?raw", import: "default", eager: true });

describe("openTags and contrast", () => {
  it("finds a tag whose attributes hold braces, quotes and a >", () => {
    const t = openTags(`<div a={() => x > 1} title="a>b" onClick={f}>hi</div>`, "div");
    expect(t).toHaveLength(1);
    expect(t[0].open.endsWith("onClick={f}>")).toBe(true);
  });
  it("does not mistake <divider> for <div>", () => {
    expect(openTags("<divider x />", "div")).toHaveLength(0);
  });
  it("computes the WCAG ratio", () => {
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 0);
    expect(contrast("#777777", "#777777")).toBe(1);
  });
});

describe("clickable markup", () => {
  it("a div, span or li with onClick has a role (or is made keyboard-operable with press)", () => {
    const bad: string[] = [];
    for (const [f, src] of Object.entries(sources)) {
      for (const tag of ["div", "span", "li"]) {
        for (const t of openTags(src, tag))
          if (/\sonClick=/.test(t.open) && !/\srole=|\{\.\.\.press\(/.test(t.open)) bad.push(`${f}:${t.line}`);
      }
    }
    expect(bad).toEqual([]);
  });

  it("no tab is a div or span (tabs are buttons, so a close button never nests inside one)", () => {
    const bad: string[] = [];
    for (const [f, src] of Object.entries(sources)) {
      for (const tag of ["div", "span"]) {
        for (const t of openTags(src, tag)) if (/role="tab"/.test(t.open)) bad.push(`${f}:${t.line}`);
      }
    }
    expect(bad).toEqual([]);
  });
});

describe("styles", () => {
  const token = (name: string) => new RegExp(name + ":\\s*(#[0-9a-fA-F]{6})").exec(css)![1];
  // text on the backgrounds it appears on: page, panel, dialog, hovered row, active row
  const BACKGROUNDS = ["#1e1f23", "#24262c", "#2a2a32", "#2b2e36", "#333746"];

  it("--muted text is at least 4.5:1 on every background it appears on", () => {
    const muted = token("--muted");
    for (const bg of BACKGROUNDS) expect(contrast(muted, bg), `${muted} on ${bg}`).toBeGreaterThanOrEqual(4.5);
  });

  // the first block of the stylesheet is the dark theme, the data-theme="light" block the light one
  const light = /:root\[data-theme="light"\]\s*\{([^}]*)\}/.exec(css)![1];
  const lightToken = (name: string) => new RegExp(name + ":\\s*(#[0-9a-fA-F]{6})").exec(light)![1];

  it("--ok, --warn and --danger are at least 4.5:1 on --panel in both themes", () => {
    for (const name of ["--ok", "--warn", "--danger"]) {
      expect(contrast(token(name), token("--panel")), `dark ${name}`).toBeGreaterThanOrEqual(4.5);
      expect(contrast(lightToken(name), lightToken("--panel")), `light ${name}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("light --muted is readable on the light surfaces", () => {
    for (const bg of ["--bg", "--panel", "--s1", "--s2"])
      expect(contrast(lightToken("--muted"), lightToken(bg)), bg).toBeGreaterThanOrEqual(4.5);
  });

  it("finished worktree rows do not fade with opacity", () => {
    expect(/\.item\.wt\.finished\s*\{[^}]*opacity/.test(css)).toBe(false);
  });

  it("the project letter is readable on the accent", () => {
    expect(contrast(token("--on-accent"), token("--accent"))).toBeGreaterThanOrEqual(4.5);
    expect(/\.picon\.letter[^}]*color: var\(--on-accent\)/.test(css)).toBe(true);
  });

  it("control borders stand out from the dialog", () => {
    expect(contrast(token("--line-ctl"), "#2a2a32")).toBeGreaterThanOrEqual(2.8);
  });

  it("motion stops under prefers-reduced-motion for everything", () => {
    expect(
      /@media \(prefers-reduced-motion: reduce\) \{\s*\*, \*::before, \*::after \{[^}]*animation-duration: 0\.001ms !important/.test(
        css,
      ),
    ).toBe(true);
  });
});

// the matrix: every text token on every surface it is drawn on, in both themes
describe("contrast matrix", () => {
  const dark = /:root \{([^}]*)\}/.exec(css)![1];
  const lightBlock = /:root\[data-theme="light"\]\s*\{([^}]*)\}/.exec(css)![1];
  const read = (block: string, into: Record<string, string>) => {
    for (const m of block.matchAll(/(--[\w-]+):\s*(#[0-9a-fA-F]{6})\b/g)) into[m[1]] = m[2];
    return into;
  };
  const themes: Record<string, Record<string, string>> = {
    dark: read(dark, {}),
    light: read(lightBlock, read(dark, {})),
  };
  const SURFACES = ["--bg", "--panel", "--s1", "--s2", "--s3"];
  // pairs that are drawn together: [text, background, minimum]
  const PAIRS: [string, string, number][] = [
    ["--ok", "--ok-bg", 4.5],
    ["--danger", "--err-bg", 4.5],
    ["--err-fg", "--err-bg", 4.5],
    ["--warn", "--warn-bg", 4.5],
    ["--accent", "--accent-bg", 4.5],
    ["--on-accent", "--accent", 4.5],
    ["--on-accent", "--danger", 4.5],
    ["--text", "--ok-bg", 4.5],
    ["--text", "--warn-bg", 4.5],
    ["--line-ctl", "--s3", 3],
    ["--line-ctl", "--bg", 3],
  ];
  for (const theme of ["dark", "light"]) {
    const t = themes[theme];
    it(`${theme}: text tokens are at least 4.5:1 on every surface`, () => {
      const bad: string[] = [];
      for (const fg of ["--text", "--muted", "--accent", "--ok", "--warn", "--danger"]) {
        for (const bg of SURFACES) {
          const r = contrast(t[fg], t[bg]);
          if (r < 4.5) bad.push(`${fg} on ${bg}: ${r.toFixed(2)}`);
        }
      }
      expect(bad).toEqual([]);
    });
    it(`${theme}: status colours are at least 4.5:1 on the page, panel and dialog`, () => {
      const bad: string[] = [];
      for (const fg of ["--await", "--stall", "--orange", "--blue"]) {
        for (const bg of ["--bg", "--panel", "--s3"]) {
          const r = contrast(t[fg], t[bg]);
          if (r < 4.5) bad.push(`${fg} on ${bg}: ${r.toFixed(2)}`);
        }
      }
      expect(bad).toEqual([]);
    });
    it(`${theme}: tinted and control pairs`, () => {
      const bad: string[] = [];
      for (const [fg, bg, min] of PAIRS) {
        const r = contrast(t[fg], t[bg]);
        if (r < min) bad.push(`${fg} on ${bg}: ${r.toFixed(2)} < ${min}`);
      }
      expect(bad).toEqual([]);
    });
  }
  it("no alias token is left: --err, --ok2 and --claude are gone", () => {
    expect(/(^|\s)--(err|ok2|claude)\s*:/m.test(css)).toBe(false);
    const src = import.meta.glob<string>(["./**/*.tsx", "./*.css"], { query: "?raw", import: "default", eager: true });
    const bad = Object.entries(src)
      .filter(([, t]) => /var\(--(err|ok2|claude)\)/.test(t))
      .map(([f]) => f);
    expect(bad).toEqual([]);
  });
});
