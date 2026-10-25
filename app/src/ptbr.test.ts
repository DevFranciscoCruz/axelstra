import { afterEach, describe, expect, it } from "vitest";
import { leafPaths, setLocale } from "./i18n";
import { parts } from "./msg";
import { S, ptS } from "./strings";
import { ptBRS } from "./strings.ptbr";
import { B, ptB } from "./strings.budget";
import { ptBRB } from "./strings.budget.ptbr";
import { L, ptL } from "./strings.logic";
import { ptBRL } from "./strings.logic.ptbr";
import { P, ptP } from "./strings.panels";
import { ptBRP } from "./strings.panels.ptbr";
import { R, ptR } from "./strings.remote";
import { ptBRR } from "./strings.remote.ptbr";
import { U, ptU } from "./strings.ui";
import { ptBRU } from "./strings.ui.ptbr";
import { W, ptW } from "./strings.welcome";
import { ptBRW } from "./strings.welcome.ptbr";
import { WS, ptWS } from "./strings.workspace";
import { ptBRWS } from "./strings.workspace.ptbr";
import { AP, ptAP } from "./strings.sources";
import { ptBRAP } from "./strings.sources.ptbr";

afterEach(() => setLocale("pt-PT"));

type Entry = [name: string, live: object, pt: object, ptBR: object];
const dicts: Entry[] = [
  ["S", S, ptS, ptBRS],
  ["P", P, ptP, ptBRP],
  ["B", B, ptB, ptBRB],
  ["R", R, ptR, ptBRR],
  ["W", W, ptW, ptBRW],
  ["WS", WS, ptWS, ptBRWS],
  ["L", L, ptL, ptBRL],
  ["U", U, ptU, ptBRU],
  ["AP", AP, ptAP, ptBRAP],
];

const at = (o: unknown, path: string): unknown =>
  path.split(".").reduce<unknown>((x, k) => (x as Record<string, unknown>)?.[k], o);

const L_ = "\\p{L}\\p{N}_-";
/** European-Portuguese-only wording that must not appear in Brazilian text: [regex source, what to write instead] */
const PT_PT_ONLY: [string, string][] = [
  ["ficheiros?", "arquivo"],
  ["ecrãs?", "tela"],
  ["utilizador(?:es)?", "usuário"],
  ["utilizar|utilizad[oa]s?", "usar"],
  ["separador(?:es)?", "aba"],
  ["ratos?", "mouse"],
  ["guardar|guardad[oa]s?|guardou|guardam|guarda", "salvar"],
  ["definições|definição", "configurações"],
  ["ligações|ligação", "conexão / link"],
  ["transferir|transferid[oa]s?", "baixar"],
  ["descarregar|descarregad[oa]s?|descarrega(?:-o)?", "baixar"],
  ["partilhar|partilhad[oa]s?", "compartilhar"],
  ["palavras?-passe", "senha"],
  ["anfitri(?:ão|ões)", "host"],
  ["contentor(?:es)?", "contêiner"],
  ["equipas?", "equipe / time"],
  ["telemóveis|telemóvel", "celular"],
  ["registos?", "log"],
  ["apagar|apagad[oa]s?|apaga", "excluir"],
  ["demasiad[oa]s?", "demais / muito"],
  ["detetar|detetad[oa]s?", "detectar"],
  ["tipo de letra", "fonte"],
  ["cópias? de segurança", "backup"],
  ["por omissão", "padrão"],
  ["gerir|gestor", "gerenciar / gerenciador"],
  ["teus?|tuas?|contigo", "seu / sua"],
  ["à espera", "aguardando"],
  ["a correr", "rodando / em execução"],
  ["sítios?", "site / lugar"],
  ["anónim[oa]s?", "anônimo"],
];
const GERUND =
  "carregar|gravar|ler|listar|procurar|pesquisar|calcular|gerar|criar|remover|importar|adicionar|verificar|transcrever|ouvir|trabalhar|instalar|reiniciar|reconectar|ligar|aguardar|dormir|iniciar";
PT_PT_ONLY.push([`a (?:${GERUND})`, "gerúndio (Carregando…)"]);
/** verbs that also read right after a noun ("commits a enviar"): only flagged when a sentence starts with them */
const SENTENCE_START = /(?<=^|[.:;…·\n]\s?)A (?:copiar|enviar|executar|atualizar|ver)/gu;

const banned = [
  ...PT_PT_ONLY.map(([src, use]) => ({ re: new RegExp(`(?<![${L_}])(?:${src})(?![${L_}])`, "giu"), use })),
  { re: SENTENCE_START, use: "gerúndio (Executando…)" },
];
/** `path: text` that may legitimately hold a word of the list above (none today) */
const ALLOWED = new Set<string>();

const offenders = (where: string, text: string): string[] =>
  banned.flatMap(({ re, use }) =>
    [...text.matchAll(re)]
      .filter((m) => !ALLOWED.has(`${where}:${m[0].toLowerCase()}`))
      .map((m) => `${where}: «${m[0]}» (use ${use})`),
  );

/** every text of a layer, for the word check: strings as they are, functions through their source (their literals), arrays as JSON */
const texts = (layer: object): [string, string][] =>
  leafPaths(layer).map((p) => {
    const v = at(layer, p);
    return [p, typeof v === "function" ? String(v) : typeof v === "string" ? v : JSON.stringify(v)];
  });

describe("pt-BR layers are complete", () => {
  for (const [name, , pt, ptBR] of dicts) {
    it(`${name}: same keys, same kinds, no empty text`, () => {
      expect(leafPaths(ptBR).sort()).toEqual(leafPaths(pt).sort());
      for (const p of leafPaths(pt)) {
        const a = at(pt, p);
        const b = at(ptBR, p);
        expect(typeof b, p).toBe(typeof a);
        if (typeof b === "string" && a !== "") expect(b.trim(), p).not.toBe("");
        if (typeof b === "function")
          expect((b as (...x: never[]) => unknown).length, p).toBe((a as (...x: never[]) => unknown).length);
        if (Array.isArray(a)) expect(Array.isArray(b) && b.length === a.length, p).toBe(true);
      }
    });
    it(`${name}: placeholders of plain texts match pt-PT`, () => {
      const names = (t: string) => [...t.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
      for (const p of leafPaths(pt)) {
        const a = at(pt, p);
        if (typeof a === "string") expect(names(at(ptBR, p) as string), p).toEqual(names(a));
      }
    });
    it(`${name}: no European-Portuguese-only wording`, () => {
      const bad = texts(ptBR).flatMap(([p, t]) => offenders(`${name}.${p}`, t));
      expect(bad).toEqual([]);
    });
  }
});

describe("pt-BR is what the app reads in that locale", () => {
  for (const [name, live, , ptBR] of dicts) {
    it(`${name}: every text reads the pt-BR one`, () => {
      setLocale("pt-BR");
      const wrong = leafPaths(ptBR).filter((p) => {
        const v = at(ptBR, p);
        return typeof v === "string" && at(live, p) !== v;
      });
      expect(wrong).toEqual([]);
    });
    it(`${name}: a function reads the pt-BR one too`, () => {
      setLocale("pt-BR");
      const fns = leafPaths(ptBR).filter((p) => typeof at(ptBR, p) === "function");
      for (const p of fns) expect(at(live, p), p).toBe(at(ptBR, p));
    });
  }
  it("uses Brazilian wording where pt-PT differs (sample)", () => {
    setLocale("pt-BR");
    expect(S.settings).toBe("Configurações");
    expect(S.loading).toBe("Carregando…");
    expect(S.app.closeFile).toBe("Fechar arquivo");
    expect(S.app.tabs).toBe("Abas");
    expect(P.confirm.save).toBe("Salvar");
    expect(P.docker.nav.containers).toBe("Contêineres");
    expect(R.reconnect).toBe("Reconectar");
    expect(L.cmd.rename.label).toBe("Renomear a aba");
    expect(U.rightTabs.files).toBe("Arquivos");
    expect(W.pickTitle).toBe("Escolha um worktree");
    expect(WS.kinds.settings).toBe("Configurações");
    expect(B.saved).toBe("Orçamentos salvos.");
    setLocale("pt-PT");
    expect(S.settings).toBe("Definições");
    expect(S.loading).toBe("A carregar…");
  });
});

describe("message texts (app/src/msg) in pt-BR", () => {
  const entries = Object.entries(parts).flatMap(([file, p]) =>
    Object.entries(p.ptBR as Record<string, string>).map(([code, t]) => [file, code, t] as const),
  );
  it("have no European-Portuguese-only wording", () => {
    const bad = entries.flatMap(([file, code, t]) => offenders(`msg/${file}.${code}`, t));
    expect(bad).toEqual([]);
  });
});
