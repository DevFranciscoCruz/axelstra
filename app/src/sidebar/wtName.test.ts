import { describe, expect, it } from "vitest";
import { wtNameProblem } from "./wtName";

describe("wtNameProblem", () => {
  it("accepts ordinary names and ignores an empty one", () => {
    expect(wtNameProblem("fix-login", [])).toBeNull();
    expect(wtNameProblem("feature/login", [])).toBeNull();
    expect(wtNameProblem("   ", [])).toBeNull();
  });
  it("refuses what git refuses as a branch name", () => {
    for (const bad of ["a b", "a~1", "a^", "a:b", "a?", "a*", "a[0", "a\b", "-x", ".x", "x.", "x/", "x.lock", "@"])
      expect(wtNameProblem(bad, []), bad).toBe("chars");
    expect(wtNameProblem("a..b", [])).toBe("dots");
    expect(wtNameProblem("a@{1}", [])).toBe("dots");
  });
  it("refuses a name already taken, as folder, branch or ax/ branch, ignoring case", () => {
    expect(wtNameProblem("login", ["login"])).toBe("exists");
    expect(wtNameProblem("Login", ["ax/login"])).toBe("exists");
    expect(wtNameProblem("login2", ["login"])).toBeNull();
  });
});
