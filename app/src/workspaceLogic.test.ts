import { describe, expect, it } from "vitest";
import { WS } from "./strings.workspace";
import { WorkspacePreview, WsSection, canImport, mappable, reasonText, sectionLine } from "./workspaceLogic";

const sec = (o: Partial<WsSection> = {}): WsSection => ({
  new: [],
  present: [],
  conflict: [],
  skipped: [],
  local_only: 0,
  ...o,
});
const preview = (o: Partial<WorkspacePreview> = {}): WorkspacePreview => ({
  version: 1,
  exported_at: "2026-10-24T10:00:00Z",
  projects: sec(),
  automations: sec(),
  settings: sec(),
  cards: sec(),
  risky_cards: 0,
  map: [],
  file: "ws.json",
  ...o,
});

describe("workspace import logic", () => {
  it("summarises a section in pt-PT with agreement in number", () => {
    expect(sectionLine(sec())).toBe(WS.nothing);
    expect(
      sectionLine(
        sec({
          new: ["a"],
          present: ["b"],
          conflict: ["c"],
          skipped: [{ label: "d", reason: "missing" }],
          local_only: 1,
        }),
      ),
    ).toBe("1 novo · 1 já existe igual · 1 diferente · 1 ignorado · 1 só existe aqui");
    expect(sectionLine(sec({ new: ["a", "b"], present: ["c", "d"], local_only: 3 }))).toBe(
      "2 novos · 2 já existem iguais · 3 só existem aqui",
    );
  });

  it("explains the skip reasons the daemon gives, and has a fallback", () => {
    for (const r of ["missing", "forbidden", "duplicate", "project skipped", "unsafe"])
      expect(reasonText(r)).not.toBe(WS.reasonOther);
    expect(reasonText("whatever")).toBe(WS.reasonOther);
  });

  it("merge needs nothing typed, replace needs the word, and nothing goes while busy", () => {
    expect(canImport("merge", "", false)).toBe(true);
    expect(canImport("merge", "", true)).toBe(false);
    expect(canImport("replace", "", false)).toBe(false);
    expect(canImport("replace", "sim", false)).toBe(false);
    expect(canImport("replace", "  Substituir ", false)).toBe(true);
    expect(canImport("replace", "substituir", true)).toBe(false);
  });

  it("offers a folder map once per missing path", () => {
    const pv = preview({
      projects: sec({
        skipped: [
          { label: "a", reason: "missing", path: "/old/a" },
          { label: "a again", reason: "missing", path: "/old/a" },
          { label: "b", reason: "forbidden", path: "/home" },
          { label: "c", reason: "missing" },
        ],
      }),
    });
    expect(mappable(pv).map((s) => s.path)).toEqual(["/old/a"]);
  });

  it("warns about risky cards in the right number", () => {
    expect(WS.risky(1)).toContain("1 cartão corre");
    expect(WS.risky(2)).toContain("2 cartões correm");
  });

  it("reports what was done", () => {
    const o = {
      projects_added: 1,
      automations_added: 1,
      automations_updated: 1,
      settings_set: 3,
      cards_written: 1,
      cards_kept: 2,
    };
    expect(WS.done(o)).toBe("1 projeto novo · 2 automatizações · 3 definições · 1 cartão escrito, 2 mantidos");
  });
});
