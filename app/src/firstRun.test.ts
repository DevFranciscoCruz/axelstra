import { describe, expect, it } from "vitest";

const src = import.meta.glob<string>("./Sidebar.tsx", { query: "?raw", import: "default", eager: true });

describe("sidebar empty state", () => {
  it("the sidebar shows nothing for an empty project list (the main area has Welcome)", () => {
    const t = Object.values(src)[0];
    expect(t).not.toContain("first-run");
    expect(t).not.toContain("Adicionar o primeiro projeto");
    expect(t).not.toMatch(/<div>Nenhum projeto<\/div>/);
  });
});
