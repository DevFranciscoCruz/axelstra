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
    expect(t[0].open.endsWith('onClick={f}>')).toBe(true);
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
        for (const t of openTags(src, tag)) if (/\sonClick=/.test(t.open) && !/\srole=|\{\.\.\.press\(/.test(t.open)) bad.push(`${f}:${t.line}`);
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

  it("the project letter is readable on the accent", () => {
    expect(contrast(token("--on-accent"), token("--accent"))).toBeGreaterThanOrEqual(4.5);
    expect(/\.picon\.letter[^}]*color: var\(--on-accent\)/.test(css)).toBe(true);
  });

  it("control borders stand out from the dialog", () => {
    expect(contrast(token("--line-ctl"), "#2a2a32")).toBeGreaterThanOrEqual(2.8);
  });

  it("motion stops under prefers-reduced-motion for everything", () => {
    expect(/@media \(prefers-reduced-motion: reduce\) \{\s*\*, \*::before, \*::after \{[^}]*animation-duration: 0\.001ms !important/.test(css)).toBe(true);
  });
});
