import { describe, expect, it } from "vitest";
import { P } from "./strings.panels";
import { removeRisks } from "./removeRisk";
import { ago, stateInfo } from "./dockerLogic";
import { planStart } from "./kanbanLogic";
import { mergeLabel } from "./shipLogic";

const panels = Object.values(import.meta.glob<string>("./strings.panels.ts", { query: "?raw", import: "default", eager: true }))[0];
// what the owned panel files must no longer carry: Portuguese text lives in strings.panels.ts
const owned = import.meta.glob<string>(
  ["./Docker.tsx", "./docker/*.tsx", "./Kanban.tsx", "./ChangesPanel.tsx", "./PullsPanel.tsx", "./Orchestra.tsx", "./DiffView.tsx", "./CommitView.tsx", "./FilesPanel.tsx", "./ShipPanel.tsx", "./Usage.tsx", "./LimitsBar.tsx", "./Voice.tsx", "./Editor.tsx", "./NoteView.tsx", "./ProjectSettings.tsx", "./Automations.tsx", "./AgentSearch.tsx", "./terminal/createXterm.ts", "./shipLogic.ts", "./removeRisk.ts", "./dockerLogic.ts", "./kanbanLogic.ts"],
  { query: "?raw", import: "default", eager: true },
);

describe("panel strings (pt-PT)", () => {
  it("keeps accented Portuguese out of the panel sources", () => {
    const hits: string[] = [];
    for (const [f, src] of Object.entries(owned)) {
      src.split("\n").forEach((line, i) => {
        if (/^\s*(\/\/|\/\*|\*)/.test(line)) return;
        if (/[ãõçáéíóúâêôà]/i.test(line)) hits.push(`${f}:${i + 1}`);
      });
    }
    expect(hits).toEqual([]);
  });
  it("uses the glossary and the tu register in strings.panels.ts", () => {
    const bad = /["`][^"`]*(pull request|\bmerged\b|dangling|\(s\)|\(es\)|\b(Adicione|Escolha|Reinicie|Abra|Escreva|Verifique|Selecione|Confirme|Tente|Instale|Crie|Fale|Clique|Inicie)\b)/i;
    const hits = panels.split("\n").map((l, i) => [l, i + 1] as const).filter(([l]) => !/^\s*(\/\/|\/\*|\*)/.test(l) && bad.test(l));
    expect(hits).toEqual([]);
  });
  it("agrees counts in number", () => {
    expect(P.docker.stopGroupAsk(1, "g", "a")).toContain("1 contentor de");
    expect(P.docker.stopGroupAsk(3, "g", "a")).toContain("3 contentores de");
    expect(P.docker.logsLinesMatch(1)).toBe("1 linha corresponde · ");
    expect(P.kanban.doneBanner(1, "orch/x")).toBe("Terminou: 1 integrado em orch/x");
    expect(P.kanban.doneBanner(2, "orch/x")).toBe("Terminou: 2 integrados em orch/x");
    expect(P.orchestra.running(1)).toBe("1 agente auxiliar a correr");
    expect(P.orchestra.running(2)).toBe("2 agentes auxiliares a correr");
    expect(removeRisks({ dirty: 1, ahead: 2, upstream: true }, 1)).toEqual(["1 ficheiro com alterações por guardar", "2 commits por enviar", "1 terminal ou agente a correr"]);
  });
  it("keeps the texts the logic modules return", () => {
    expect(stateInfo("running").text).toBe("a correr");
    expect(stateInfo("").text).toBe("desconhecido");
    expect(ago(Date.now() - 5 * 60_000)).toBe("há 5 minutos");
    expect(ago(Date.now() - 61 * 60_000)).toBe("há 1 hora");
    expect(planStart(1, null, []).message).toContain("1 agente");
    expect(planStart(1, null, ["a: x"]).message).toContain("na tua máquina");
    expect(mergeLabel({ pr_state: "merged" } as never)).toBe("Merge feito");
  });
});
