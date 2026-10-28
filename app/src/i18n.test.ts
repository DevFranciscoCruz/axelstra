import { afterEach, describe, expect, it } from "vitest";
import { LOCALES, count, dict, fmtNumber, leafPaths, locale, plural, setLocale } from "./i18n";
import { ptB } from "./strings.budget";
import { enB } from "./strings.budget.en";
import { ptP } from "./strings.panels";
import { enP } from "./strings.panels.en";
import { ptR } from "./strings.remote";
import { enR } from "./strings.remote.en";
import { ptS } from "./strings";
import { ptL } from "./strings.logic";
import { enL } from "./strings.logic.en";
import { ptBRL } from "./strings.logic.ptbr";
import { enS } from "./strings.en";
import { ptW } from "./strings.welcome";
import { enW } from "./strings.welcome.en";
import { ptWS } from "./strings.workspace";
import { ptPF } from "./strings.perf";
import { enPF } from "./strings.perf.en";
import { enWS } from "./strings.workspace.en";
import { ptU } from "./strings.ui";
import { enU } from "./strings.ui.en";
import { ptAP } from "./strings.sources";
import { enAP } from "./strings.sources.en";

afterEach(() => setLocale("pt-PT"));

describe("dict", () => {
  const d = dict(
    { a: "um", n: { b: "dois", f: (x: number) => `x${x}` }, list: ["p"] },
    { a: "one", n: { b: "two", f: (x: number) => `y${x}` }, list: ["q"] },
    { a: "uma" },
  );
  it("reads the current language, live", () => {
    expect(d.a).toBe("um");
    setLocale("en");
    expect(d.a).toBe("one");
    expect(d.n.b).toBe("two");
    expect(d.n.f(2)).toBe("y2");
    expect(d.list).toEqual(["q"]);
    setLocale("pt-BR");
    expect(d.a).toBe("uma");
    expect(d.n.b).toBe("dois"); // pt-BR falls back to pt-PT for what it does not override
  });
  it("keeps the keys of the source for iteration", () => {
    setLocale("en");
    expect(Object.entries(d.n).map(([k, v]) => [k, typeof v])).toEqual([
      ["b", "string"],
      ["f", "function"],
    ]);
  });
});

describe("locale-aware plurals and numbers", () => {
  it("counts with the words of the language", () => {
    expect(count(1, "worktree", "worktrees")).toBe("1 worktree");
    expect(count(0, "worktree", "worktrees")).toBe("0 worktrees");
    setLocale("pt-BR");
    expect(plural(0, "um", "vários")).toBe("um"); // pt-BR counts 0 as singular
    setLocale("pt-PT");
    expect(plural(0, "um", "vários")).toBe("vários");
  });
  it("formats numbers with the locale", () => {
    setLocale("en");
    expect(fmtNumber(1234.5)).toBe("1,234.5");
    setLocale("pt-PT");
    expect(fmtNumber(1234.5)).toBe("1234,5");
    expect(LOCALES).toContain(locale());
  });
});

/** words that are the same in every language (names, protocol terms) and so legitimately equal in both dictionaries */
const SAME =
  /^(?:|[\s·/.:;,…()[\]{}<>+\-–—%|$#@*&=^~"'`«»\d]+|git|gh|GitHub|PR|PRs|Docker|Kanban|Compose|CPU|ctx|Claude|Codex|Gemini|Axelstra|daemon|WSL|CMD|Bash|PowerShell|npm|pnpm|yarn|bun|token|tokens|Terminal|Shell|Worktrees?|Hosts?|PowerShell 7|Windows PowerShell|Claude Code|Gemini CLI|branch|commit|merge|push|pull|rebase|stash|diff|SSH|URL|HTTPS?|JSON|YAML|OK|Ok|ok|n\/a|Editor|Volumes|Info|Logs|Total|Checks|CI|base|bin|main|COMMITS|Auto-merge|Compose down|Commit \(Ctrl\+Enter\))$/i;

type Pair = [string, object, object];
const pairs: Pair[] = [
  ["S", ptS, enS],
  ["P", ptP, enP],
  ["B", ptB, enB],
  ["R", ptR, enR],
  ["W", ptW, enW],
  ["WS", ptWS, enWS],
  ["PF", ptPF, enPF],
  ["L", ptL, enL],
  ["U", ptU, enU],
  ["AP", ptAP, enAP],
];

const at = (o: unknown, path: string): unknown =>
  path.split(".").reduce<unknown>((x, k) => (x as Record<string, unknown>)?.[k], o);

describe("every locale has every key", () => {
  for (const [name, pt, en] of pairs) {
    it(`${name}: English has the same keys as pt-PT`, () => {
      expect(leafPaths(en).sort()).toEqual(leafPaths(pt).sort());
    });
    it(`${name}: functions and arrays keep their kind`, () => {
      for (const p of leafPaths(pt)) expect(typeof at(en, p), p).toBe(typeof at(pt, p));
    });
    it(`${name}: English is translated, not a copy of pt-PT`, () => {
      const copies = leafPaths(pt).filter((p) => {
        const a = at(pt, p);
        return typeof a === "string" && a === at(en, p) && !SAME.test(a.trim());
      });
      expect(copies).toEqual([]);
    });
  }
});

describe("pt-BR", () => {
  it("overrides only keys that exist in pt-PT, with the same kind of value", () => {
    const stray = leafPaths(ptBRL).filter((p) => typeof at(ptL, p) !== typeof at(ptBRL, p));
    expect(stray).toEqual([]);
  });
});
