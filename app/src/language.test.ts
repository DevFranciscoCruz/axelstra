import { describe, expect, it } from "vitest";

const sources = import.meta.glob<string>(["./*.tsx", "./strings.ts", "./store.ts", "./projectActions.ts", "./agentState.ts"], { query: "?raw", import: "default", eager: true });
// files owned by other work areas are not held to this list here
const OTHERS = /Kanban|Orchestra|Usage|StatusBar|LimitsBar/;

const scan = (re: RegExp) => {
  const hits: string[] = [];
  for (const [f, src] of Object.entries(sources)) {
    if (OTHERS.test(f)) continue;
    src.split("\n").forEach((line, i) => {
      if (/^\s*(\/\/|\/\*|\*)/.test(line)) return;
      if (re.test(line)) hits.push(`${f}:${i + 1}`);
    });
  }
  return hits;
};

describe("UI language (pt-PT)", () => {
  it("speaks to the user in the tu register, not the formal one", () => {
    expect(scan(/"[^"]*\b(Adicione|Escolha|Reinicie|Reinicie-o|Peça|Desligue|Abra|Escreva|Verifique|Selecione|Confirme|Tente|Se preferir)\b[^"]*"/)).toEqual([]);
  });
  it("has no stray English in what the user reads", () => {
    expect(scan(/"[^"]*\((detached)\)[^"]*"|>\s*Issues\s*<|"Issues"/)).toEqual([]);
  });
});
