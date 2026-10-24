import { describe, expect, it } from "vitest";

const src = import.meta.glob<string>("./Sidebar.tsx", { query: "?raw", import: "default", eager: true });

describe("sidebar first run (U4)", () => {
  it("the empty project list is a first-run card, not just 'Nenhum projeto'", () => {
    const t = Object.values(src)[0];
    expect(t).toContain('aria-label="Primeiros passos"');
    expect(t).toContain("Bem-vindo ao Axelstra");
    expect(t).not.toMatch(/<div>Nenhum projeto<\/div>/);
  });
});
