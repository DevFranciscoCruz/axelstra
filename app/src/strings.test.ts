import { describe, expect, it } from "vitest";
import { S } from "./strings";

describe("deletion notices", () => {
  it("name what was deleted", () => {
    expect(S.fileDeleted("a.ts")).toBe("Ficheiro apagado: a.ts");
    expect(S.folderDeleted("src")).toBe("Pasta apagada: src");
    expect(S.cardDeleted("Corrigir")).toBe("Cartão apagado: Corrigir");
    expect(S.automationDeleted("Noite")).toBe("Automatização apagada: Noite");
    expect(S.worktreeRemoved("ax/x")).toBe("Worktree removido: ax/x");
    expect(S.projectRemoved("axelstra")).toBe("Projeto tirado da lista: axelstra");
  });
  it("failure names the item and keeps the error text", () => {
    expect(S.deleteFailed("a.ts", new Error("acesso negado"))).toBe('Não foi possível apagar "a.ts": acesso negado');
    expect(S.deleteFailed("a.ts", "boom")).toBe('Não foi possível apagar "a.ts": boom');
  });
});
