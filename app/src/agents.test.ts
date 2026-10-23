import { describe, expect, it } from "vitest";
import { AgentInfo, agentLine, agentOptions, agentWarning } from "./api";

const ag = (id: string, name: string, installed: boolean, install = "npm i -g x"): AgentInfo => ({
  id, name, installed, path: installed ? `/bin/${id}` : null, install, can_resume: true, interactive_prompt: true,
});
const list = [ag("claude", "Claude Code", true), ag("codex", "Codex", true), ag("gemini", "Gemini CLI", false, "npm install -g @google/gemini-cli")];

describe("agent pickers", () => {
  it("offers only installed agents", () => {
    expect(agentOptions(list).map((o) => o.value)).toEqual(["claude", "codex"]);
  });
  it("keeps a saved choice that is not installed, marked", () => {
    expect(agentOptions(list, "gemini").at(-1)).toEqual({ value: "gemini", label: "Gemini CLI (não instalado)" });
    expect(agentOptions(list, "zzz").at(-1)?.label).toBe("zzz (desconhecido)");
  });
  it("treats the old chatgpt id as codex", () => {
    expect(agentOptions(list, "chatgpt").map((o) => o.value)).toEqual(["claude", "codex"]);
    expect(agentWarning(list, "chatgpt")).toBe("");
  });
  it("warns in pt-PT with the install command", () => {
    expect(agentWarning(list, "gemini")).toBe("Gemini CLI não encontrado: instala com npm install -g @google/gemini-cli");
    expect(agentWarning(list, "claude")).toBe("");
    expect(agentWarning([], "gemini")).toBe("");
  });
});

describe("agentLine resume", () => {
  it("continues the latest conversation per agent", () => {
    expect(agentLine("gemini", undefined, true, false, "h")).toBe("gemini --resume latest || gemini");
    expect(agentLine("opencode", undefined, true, false, "h")).toBe("opencode --continue || opencode");
    expect(agentLine("codex", undefined, true, false, "h")).toBe("codex resume --last || codex");
    expect(agentLine("aider", undefined, true, false, "h")).toBe("aider || aider");
  });
});
