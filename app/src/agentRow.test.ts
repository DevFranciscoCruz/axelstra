import { describe, expect, it, vi } from "vitest";

vi.mock("./store", () => ({ norm: (p?: string | null) => (p ?? "").replace(/\\/g, "/").replace(/\/$/, "").toLowerCase() }));

import { agentRowInfo, agentRowLabel, folderName, overlapsOf, overlapTitle } from "./agentRow";
import { parseNote } from "./noteMd";

const sess = (over: object = {}) => ({ id: 4, label: "Tarefa 2", meta: { kind: "agent", agent: "claude", worktree: "C:\\w\\proj-a", note: "C:/d/notes/p/Tarefa 2.md" }, ...over });

describe("agent row", () => {
  it("shows the name, the kind tag and no worktree when it is the listed one", () => {
    const i = agentRowInfo(sess(), "c:/w/proj-a/");
    expect(i).toEqual({ name: "Tarefa 2", kind: "claude", worktree: "", hasReport: true });
  });
  it("names the worktree of a worker that runs elsewhere", () => {
    expect(agentRowInfo(sess(), "C:/w/other").worktree).toBe("proj-a");
  });
  it("falls back to the session number and has no report without a task file", () => {
    const i = agentRowInfo(sess({ label: null, meta: { kind: "agent" } }), "C:/w/x");
    expect(i.name).toBe("Sessão 4");
    expect(i.hasReport).toBe(false);
  });
  it("prefers the name the user just typed", () => {
    expect(agentRowInfo(sess(), "C:/w/proj-a", "Corrigir login").name).toBe("Corrigir login");
  });
  it("reads as one phrase", () => {
    expect(agentRowLabel(agentRowInfo(sess(), "C:/w/other"), "a trabalhar")).toBe("Tarefa 2, claude, em proj-a, a trabalhar");
  });
  it("folderName handles both slashes and a trailing one", () => {
    expect(folderName("C:\\a\\b\\")).toBe("b");
    expect(folderName(null)).toBe("");
  });
});

describe("overlap badge", () => {
  it("builds the tooltip from the other worktrees", () => {
    expect(overlapTitle(["C:/w/a", "C:\\w\\b"])).toBe("Edita os mesmos ficheiros que: a, b");
    expect(overlapTitle([])).toBeNull();
    expect(overlapTitle(undefined)).toBeNull();
  });
  it("finds a worktree whatever the slash style or case of the key", () => {
    const map = { "C:\\W\\One": ["C:/w/two"] };
    expect(overlapsOf(map, "c:/w/one/")).toEqual(["C:/w/two"]);
    expect(overlapsOf(map, "c:/w/none")).toBeUndefined();
    expect(overlapsOf(null, "c:/w/one")).toBeUndefined();
  });
});

describe("task file view", () => {
  it("parses what the daemon writes", () => {
    const md = "# Tarefa 1\n\n- Agente: claude\n- Worktree: C:/x\n\n## Objetivo\n\nfaz <isto>\n& aquilo\n\n## Relatorio\n\n### 2026-10-23 10:00 - ok\n\nfeito\n";
    const b = parseNote(md);
    expect(b[0]).toEqual({ kind: "h", level: 1, text: "Tarefa 1" });
    expect(b.filter((x) => x.kind === "li")).toHaveLength(2);
    expect(b).toContainEqual({ kind: "p", text: "faz <isto>\n& aquilo" });
    expect(b).toContainEqual({ kind: "h", level: 3, text: "2026-10-23 10:00 - ok" });
    expect(b.at(-1)).toEqual({ kind: "p", text: "feito" });
  });
  it("keeps code fences whole, even unclosed", () => {
    expect(parseNote("```\na\n\n# not a heading\n```")).toEqual([{ kind: "code", text: "a\n\n# not a heading" }]);
    expect(parseNote("```\nopen")).toEqual([{ kind: "code", text: "open" }]);
  });
  it("handles windows line endings and empty text", () => {
    expect(parseNote("a\r\nb\r\n")).toEqual([{ kind: "p", text: "a\nb" }]);
    expect(parseNote("")).toEqual([]);
  });
});
