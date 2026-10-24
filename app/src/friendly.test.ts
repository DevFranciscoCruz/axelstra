import { describe, expect, it } from "vitest";
import { friendlyError } from "./friendly";
import { count, plural } from "./strings";

describe("friendlyError", () => {
  it("prefixes an unknown git/gh/docker message with a Portuguese line and keeps the original", () => {
    for (const m of ["fatal: something exotic happened", "error: pack-objects died", "Error response from daemon: weird", "gh: Could not do the thing"]) {
      const out = friendlyError(m);
      expect(out).toMatch(/^Uma ferramenta externa/);
      expect(out).toContain(m);
    }
  });
  it("translates more known docker/gh/git failures", () => {
    expect(friendlyError("Cannot connect to the Docker daemon at npipe")).toMatch(/Docker não está a correr/);
    expect(friendlyError("Bind for 0.0.0.0:80 failed: port is already allocated")).toMatch(/porta/);
    expect(friendlyError("To continue, run gh auth login")).toMatch(/gh auth login/);
    expect(friendlyError("CONFLICT: Automatic merge failed")).toMatch(/conflitos/);
  });
  it("leaves ordinary messages alone", () => {
    expect(friendlyError("Algo correu mal")).toBe("Algo correu mal");
  });
});

describe("plural helpers", () => {
  it("agree in number the pt-PT way", () => {
    expect(plural(1, "worktree", "worktrees")).toBe("worktree");
    expect(plural(0, "worktree", "worktrees")).toBe("worktrees");
    expect(plural(2, "worktree", "worktrees")).toBe("worktrees");
    expect(count(1, "sessão", "sessões")).toBe("1 sessão");
    expect(count(3, "sessão", "sessões")).toBe("3 sessões");
  });
});
