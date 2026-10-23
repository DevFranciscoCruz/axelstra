import { describe, expect, it } from "vitest";
import { agentLine, geminiPathVariants, geminiResumeId, hitResume } from "./api";

describe("agent search hits", () => {
  it("resumes claude by session id and gemini by a tagged id; codex cannot", () => {
    expect(hitResume({ agent: "claude", session: "abc" })).toBe("abc");
    expect(hitResume({ agent: "gemini", session: "7d9c1c0e-2b0a" })).toBe("gemini:7d9c1c0e-2b0a");
    expect(hitResume({ agent: "codex", session: "x" })).toBeNull();
  });
  it("only accepts a plain id after the gemini prefix", () => {
    expect(geminiResumeId("gemini:7d9c1c0e-2b0a")).toBe("7d9c1c0e-2b0a");
    expect(geminiResumeId("gemini:a; rm -rf /")).toBeNull();
    expect(geminiResumeId("gemini:")).toBeNull();
    expect(geminiResumeId("some-tab-key")).toBeNull();
    expect(geminiResumeId(undefined)).toBeNull();
  });
  it("builds the resume line for a chosen gemini conversation and keeps latest for the rest", () => {
    expect(agentLine("gemini", "gemini:abc-1", true, false, "h.json")).toBe("gemini --resume abc-1 || gemini");
    expect(agentLine("gemini", "tab-key", true, false, "h.json")).toBe("gemini --resume latest || gemini");
    expect(agentLine("gemini", "gemini:abc-1", false, false, "h.json")).toBe("gemini");
  });
  it("lists the spellings Gemini may have hashed for a Windows path", () => {
    expect(geminiPathVariants("C:\\wt\\a\\")).toEqual(["C:\\wt\\a\\", "C:\\wt\\a", "C:/wt/a"]);
    expect(geminiPathVariants("/home/x")).toEqual(["/home/x", "\\home\\x"]);
  });
});
