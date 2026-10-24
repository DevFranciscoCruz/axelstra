import { describe, expect, it } from "vitest";
import { basename, displayPath, hostOf, isRemote, norm, remotePathOf, toggleIn } from "./paths";

describe("paths helpers", () => {
  it("basename takes the last segment with either slash and ignores trailing ones", () => {
    expect(basename(String.raw`C:\a\b\\`)).toBe("b");
    expect(basename("/x/y/z")).toBe("z");
    expect(basename("solo")).toBe("solo");
    expect(basename(null)).toBe("");
  });
  it("norm compares paths in one form", () => {
    expect(norm(String.raw`C:\A\b\\`)).toBe(norm("c:/a/b"));
  });
  it("toggleIn flips membership without touching the original", () => {
    const a = new Set([1]);
    expect([...toggleIn(a, 2)]).toEqual([1, 2]);
    expect([...toggleIn(a, 1)]).toEqual([]);
    expect([...a]).toEqual([1]);
  });
  it("remote paths carry the host id and keep basename and norm working", () => {
    expect(hostOf("@lab:/home/me/proj")).toBe("lab");
    expect(remotePathOf("@lab:/home/me/proj")).toBe("/home/me/proj");
    expect(isRemote("@wsl1:/home/me/x")).toBe(true);
    expect(basename("@lab:/home/me/proj/")).toBe("proj");
    expect(norm("@lab:/home/me/proj/")).toBe("@lab:/home/me/proj");
    expect(displayPath("@lab:/home/me/proj")).toBe("lab:/home/me/proj");
    expect(displayPath("@lab:/home/me/proj", "Servidor")).toBe("Servidor:/home/me/proj");
  });
  it("local paths are not remote", () => {
    for (const p of [String.raw`C:\x\y`, "/home/me", "", "a@b:c"]) {
      expect(hostOf(p)).toBeNull();
      expect(isRemote(p)).toBe(false);
      expect(remotePathOf(p)).toBe(p);
      expect(displayPath(p)).toBe(p);
    }
    expect(hostOf(null)).toBeNull();
  });
});
