import { describe, expect, it } from "vitest";
import { COMMANDS, cheatSheet, paletteCommands } from "./commands";

describe("command registry", () => {
  it("has unique ids", () => {
    expect(new Set(COMMANDS.map((c) => c.id)).size).toBe(COMMANDS.length);
  });
  it("lists the shortcuts the old sheet missed", () => {
    const keys = cheatSheet().flatMap((g) => g.rows.map((r) => r[0]));
    for (const k of ["F2", "Ctrl+Enter", "Shift+F10", "Ctrl+F", "?"]) expect(keys).toContain(k);
  });
  it("the palette offers only commands with a handler and can relabel them", () => {
    const run = () => {};
    const out = paletteCommands({
      docker: { run },
      toggleSidebar: { run, label: "Ocultar projetos" },
      nextTab: { run },
    });
    expect(out.map((c) => c.id)).toEqual(["toggleSidebar", "docker"]);
    expect(out[0]).toMatchObject({ label: "Ocultar projetos", hint: "Ctrl+B" });
  });
  it("covers the new palette entries", () => {
    const ids = COMMANDS.filter((c) => c.palette !== false).map((c) => c.id);
    for (const id of ["docker", "notifications", "theme", "voice", "toggleRight", "restartDaemon"])
      expect(ids).toContain(id);
  });
});
