import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "./i18n";
import { ciLine, stamp, toMarkdown, tokenLine, type Timeline } from "./timelineLogic";

const t: Timeline = {
  branch: "ax/feat",
  base: "main",
  events: [
    { at: 300, kind: "report", title: "main - feat", detail: "ok · feito *tudo*" },
    { at: 200, kind: "commit", title: "fix: b", detail: "abc1234 · Ana" },
    { at: 100, kind: "commit", title: "feat: a\nsegunda linha", detail: "" },
  ],
  tokens: { input: 1500, output: 500, cache_read: 2_000_000, cache_write: 0 },
  ci: {
    number: 7,
    title: "x",
    url: "u",
    ci: "fail",
    checks: [
      { name: "build", state: "fail", link: "" },
      { name: "lint", state: "pass", link: "" },
    ],
  },
};

afterEach(() => setLocale("pt-PT"));

describe("timeline export", () => {
  it("keeps ISO times in the file in every language", () => {
    for (const l of ["pt-PT", "en", "pt-BR"] as const) {
      setLocale(l);
      expect(toMarkdown(t, "feat")).toMatch(/\*\*\d{4}-\d\d-\d\d \d\d:\d\d\*\*/);
    }
  });
  it("writes oldest first with escaped text", () => {
    const md = toMarkdown(t, "feat");
    const lines = md.split("\n");
    const first = lines.findIndex((l) => l.includes("feat: a"));
    const last = lines.findIndex((l) => l.includes("main - feat"));
    expect(first).toBeGreaterThan(0);
    expect(last).toBeGreaterThan(first);
    expect(md).toContain("feat: a segunda linha"); // newline folded so the list stays one line per event
    expect(md).toContain("\\*tudo\\*");
    expect(md).toContain("`ax/feat`");
  });
  it("lists tokens and CI in the header", () => {
    const md = toMarkdown(t, "feat");
    expect(md).toContain(tokenLine(t.tokens!));
    expect(md).toContain(ciLine(t.ci!));
    expect(ciLine(t.ci!)).toContain("build");
    expect(ciLine(t.ci!)).not.toContain("lint");
  });
  it("says so when nothing happened", () => {
    expect(toMarkdown({ ...t, events: [], tokens: null, ci: null }, "x")).toContain("Sem eventos");
  });
  it("shows the time in the UI language", () => expect(stamp(0)).toMatch(/^\d\d\/\d\d\/\d{4}, \d\d:\d\d$/));
});
