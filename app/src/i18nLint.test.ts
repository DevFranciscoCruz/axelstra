import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "./i18n";
import { COMMANDS, GROUP_LABEL, cheatSheet } from "./commands";
import { DOT_LABEL } from "./agentState";
import { friendlyError } from "./friendly";
import { ago } from "./time";
import { S } from "./strings";
import { micErrorText } from "./voiceErrors";
import { fmtTokens } from "./usageFormat";

afterEach(() => setLocale("pt-PT"));

const sources = import.meta.glob<string>(
  ["./**/*.tsx", "./**/*.ts", "!./**/*.test.ts", "!./**/strings*.ts", "!./msg/*.ts", "!./ui/Gallery.tsx"],
  { query: "?raw", import: "default", eager: true },
);

const isComment = (line: string) => /^\s*(\/\/|\/\*|\*)/.test(line);

/** words that read the same in every language: product and tool names, units, symbols */
const NEUTRAL =
  /^(?:[^A-Za-zÀ-ÿ]*|\+? ?(?:Terminal|Docker|Kanban|Compose|CPU|ctx|git|PR|PRs|Axelstra|Σ|MB|GB|Ctrl|Shift|Alt|Enter|Esc))$/;

describe("no user-visible literal outside the dictionaries", () => {
  it("attributes that the user reads go through S/P/U/...", () => {
    const re = /\b(?:title|aria-label|placeholder|alt|label|okLabel|cancelLabel|hint)="([^"{}]*)"/g;
    const hits: string[] = [];
    for (const [f, src] of Object.entries(sources)) {
      if (!f.endsWith(".tsx")) continue;
      src.split("\n").forEach((line, i) => {
        if (isComment(line)) return;
        for (const m of line.matchAll(re))
          if (/[A-Za-zÀ-ÿ]{2,}/.test(m[1]) && !NEUTRAL.test(m[1].trim())) hits.push(`${f}:${i + 1} ${m[0]}`);
      });
    }
    expect(hits).toEqual([]);
  });

  it("JSX text goes through the dictionaries", () => {
    const re = />([^<>{}=]*[A-Za-zÀ-ÿ]{2,}[^<>{}]*)<\//g;
    const hits: string[] = [];
    for (const [f, src] of Object.entries(sources)) {
      if (!f.endsWith(".tsx")) continue;
      src.split("\n").forEach((line, i) => {
        if (isComment(line)) return;
        for (const m of line.matchAll(re))
          if (!NEUTRAL.test(m[1].trim()) && !/^[\w.\-/]+$/.test(m[1].trim())) hits.push(`${f}:${i + 1} ${m[0]}`);
      });
    }
    expect(hits).toEqual([]);
  });

  it("no accented Portuguese inside a string of the code", () => {
    const re = /"[^"\n]*[ãõçáéíóúâêôà][^"\n]*"|`[^`\n]*[ãõçáéíóúâêôà][^`\n]*`/;
    const hits: string[] = [];
    for (const [f, src] of Object.entries(sources)) {
      if (f.endsWith("i18n.ts")) continue;
      src.split("\n").forEach((line, i) => {
        if (isComment(line) || /console\.|logFrontendError|\/[^/]*[ãçé][^/]*\/[gimsuy]*[.,;)]/.test(line)) return;
        if (re.test(line)) hits.push(`${f}:${i + 1} ${line.trim().slice(0, 100)}`);
      });
    }
    expect(hits).toEqual([]);
  });
});

describe("the language changes live", () => {
  it("commands, groups, dots and plain texts follow the locale", () => {
    const settings = COMMANDS.find((c) => c.id === "settings")!;
    expect([settings.label, GROUP_LABEL.geral, DOT_LABEL.working, S.settings]).toEqual([
      "Definições",
      "Geral",
      "A trabalhar",
      "Definições",
    ]);
    setLocale("en");
    expect([settings.label, GROUP_LABEL.geral, DOT_LABEL.working, S.settings]).toEqual([
      "Settings",
      "General",
      "Working",
      "Settings",
    ]);
    expect(cheatSheet()[0].title).toBe("General");
    expect(COMMANDS.find((c) => c.id === "tabClose")!.keys).toBe("Middle-click the tab");
  });
  it("friendly errors, dates and numbers follow the locale", () => {
    expect(friendlyError("fatal: not a git repository")).toContain("não é um repositório git");
    expect(ago(300)).toBe("há 5 min");
    expect(fmtTokens(1500)).toBe("1,5k");
    setLocale("en");
    expect(friendlyError("fatal: not a git repository")).toContain("not a git repository.");
    expect(ago(300)).toBe("5 min. ago");
    expect(ago(5)).toBe("now");
    expect(fmtTokens(1500)).toBe("1.5k");
    expect(micErrorText(new Error("x"))).toContain("microphone");
    setLocale("pt-BR");
    expect(ago(300)).toBe("há 5 min.");
  });
  it("pt-BR reads its own complete layer", () => {
    setLocale("pt-BR");
    expect(S.settings).toBe("Configurações");
  });
});

describe("glossary (see the comment at the top of strings.ts)", () => {
  const raw = import.meta.glob<string>(["./strings*.ts", "./msg/*.ts", "!./**/*.test.ts", "!./msg/index.ts"], {
    query: "?raw",
    import: "default",
    eager: true,
  });
  const literals = (src: string) => src.match(/"(?:[^"\\\n]|\\.)*"|`(?:[^`\\]|\\.)*`/g) ?? [];
  /** the texts of one layer: strings*.ts files are one layer each (by name); msg files hold pt, en and ptBR in one file */
  const layerTexts = (layer: "pt" | "ptBR"): [string, string][] => {
    const out: [string, string][] = [];
    for (const [f, src] of Object.entries(raw)) {
      const msg = f.startsWith("./msg/");
      const code = src.split("\n").filter((l) => !isComment(l)).join("\n");
      if (msg) {
        const en = code.search(/export const en\b/);
        const br = code.search(/export const ptBR\b/);
        if (en < 0 || br < 0) continue;
        const part = layer === "pt" ? code.slice(0, en) : code.slice(br);
        for (const t of literals(part)) out.push([f, t]);
      } else if (layer === "ptBR" ? f.includes(".ptbr.") : !/\.(en|ptbr)\.ts$/.test(f)) {
        for (const t of literals(code)) out.push([f, t]);
      }
    }
    return out;
  };
  const scan = (layer: "pt" | "ptBR", re: RegExp) =>
    layerTexts(layer)
      .filter(([, t]) => re.test(t))
      .map(([f, t]) => `${f}: ${t.slice(0, 90)}`);

  it("pt-PT texts use the glossary: enviar (not push), PR (not pull request), host, o branch", () => {
    expect(scan("pt", /\bpush\b|pull request|anfitri|\bramo\b|\b(?:a|uma|na|da|esta|nesta|essa|dessa) branch\b/i)).toEqual(
      [],
    );
  });
  it("pt-BR texts keep the glossary and avoid pt-PT wording", () => {
    expect(
      scan("ptBR", /\bpush\b|pull request|anfitri|\bramo\b|\b(?:a|uma|na|da|esta|nesta|essa|dessa) branch\b|\becrã\b|ficheiro|\ba correr\b/i),
    ).toEqual([]);
  });
});
