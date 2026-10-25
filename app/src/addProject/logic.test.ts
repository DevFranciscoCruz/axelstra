import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  SOURCES,
  depthProblem,
  filterRepos,
  joinPath,
  lastParent,
  lastSource,
  listKey,
  looksLikeFolder,
  nameProblem,
  nextSource,
  rememberParent,
  rememberSource,
  repoNameFromUrl,
  urlProblem,
} from "./logic";

// newer Node ships its own (unusable) localStorage that shadows jsdom's: use a plain in-memory one
const memory = () => {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
    clear: () => m.clear(),
  };
};

beforeEach(() => vi.stubGlobal("localStorage", memory()));

describe("repoNameFromUrl", () => {
  it("takes the last segment without .git", () => {
    expect(repoNameFromUrl("https://github.com/o/repo.git")).toBe("repo");
    expect(repoNameFromUrl("https://github.com/o/repo")).toBe("repo");
    expect(repoNameFromUrl("https://github.com/o/repo/")).toBe("repo");
    expect(repoNameFromUrl("git@github.com:o/my-repo.git")).toBe("my-repo");
    expect(repoNameFromUrl("ssh://git@host:2222/srv/a.b.git")).toBe("a.b");
    expect(repoNameFromUrl("https://host/o/repo.git?x=1#y")).toBe("repo");
  });
  it("gives nothing for what is not an address", () => {
    expect(repoNameFromUrl("")).toBe("");
    expect(repoNameFromUrl("   ")).toBe("");
    expect(repoNameFromUrl("https://")).toBe("");
    expect(repoNameFromUrl("https://host/..")).toBe("");
  });
});

describe("urlProblem", () => {
  it("accepts https, ssh, git and scp-like addresses", () => {
    for (const u of [
      "https://github.com/o/r.git",
      "ssh://git@host/o/r",
      "git@github.com:o/r.git",
      "  https://github.com/o/r  ",
    ])
      expect(urlProblem(u), u).toBeNull();
  });
  it("names what is wrong", () => {
    expect(urlProblem("")).toBe("empty");
    expect(urlProblem("https://a b/c")).toBe("space");
    expect(urlProblem("--upload-pack=evil")).toBe("dash");
    expect(urlProblem("-oProxyCommand=x")).toBe("dash");
    expect(urlProblem("ftp://host/r")).toBe("scheme");
    expect(urlProblem("file:///tmp/r")).toBe("scheme");
    expect(urlProblem("/home/me/r")).toBe("scheme");
    expect(urlProblem("justaword")).toBe("scheme");
  });
});

describe("nameProblem", () => {
  it("accepts plain folder names", () => {
    for (const n of ["app", "my-app", "my.app", "app_2", "Projeto Novo"]) expect(nameProblem(n), n).toBeNull();
  });
  it("refuses separators, dots, reserved and bad characters", () => {
    expect(nameProblem("")).toBe("empty");
    expect(nameProblem("   ")).toBe("empty");
    expect(nameProblem("a/b")).toBe("sep");
    expect(nameProblem("a\\b")).toBe("sep");
    expect(nameProblem("..")).toBe("dots");
    expect(nameProblem(".")).toBe("dots");
    expect(nameProblem("con")).toBe("reserved");
    expect(nameProblem("NUL")).toBe("reserved");
    expect(nameProblem("com1.txt")).toBe("reserved");
    expect(nameProblem("a:b")).toBe("chars");
    expect(nameProblem("a*b")).toBe("chars");
    expect(nameProblem("name.")).toBe("trail");
    expect(nameProblem("x".repeat(101))).toBe("long");
  });
});

describe("depthProblem and joinPath", () => {
  it("depth is empty or a whole number from 1", () => {
    expect(depthProblem("")).toBe(false);
    expect(depthProblem("1")).toBe(false);
    expect(depthProblem("50")).toBe(false);
    expect(depthProblem("0")).toBe(true);
    expect(depthProblem("-1")).toBe(true);
    expect(depthProblem("1.5")).toBe(true);
    expect(depthProblem("abc")).toBe(true);
  });
  it("joins with the separator the parent uses", () => {
    expect(joinPath("/home/me", "r")).toBe("/home/me/r");
    expect(joinPath("/home/me/", "r")).toBe("/home/me/r");
    expect(joinPath("C:\\Users\\me", "r")).toBe("C:\\Users\\me\\r");
    expect(joinPath("C:", "r")).toBe("C:\\r");
    expect(joinPath("", "r")).toBe("r");
  });
});

describe("filterRepos", () => {
  const repos = [
    { name_with_owner: "me/alpha", description: "A tool" },
    { name_with_owner: "me/beta", description: null },
    { name_with_owner: "org/Gamma", description: "alpha-like thing" },
  ];
  it("matches every word in name or description, ignoring case", () => {
    expect(filterRepos(repos, "").length).toBe(3);
    expect(filterRepos(repos, "ALPHA").map((r) => r.name_with_owner)).toEqual(["me/alpha", "org/Gamma"]);
    expect(filterRepos(repos, "alpha tool").map((r) => r.name_with_owner)).toEqual(["me/alpha"]);
    expect(filterRepos(repos, "zzz")).toEqual([]);
  });
});

describe("keyboard on lists", () => {
  it("listKey moves with wrap, Home and End, and activates on Enter only on a real row", () => {
    expect(listKey(-1, 3, "ArrowDown")).toEqual({ row: 0, activate: false });
    expect(listKey(2, 3, "ArrowDown")).toEqual({ row: 0, activate: false });
    expect(listKey(0, 3, "ArrowUp")).toEqual({ row: 2, activate: false });
    expect(listKey(1, 3, "End").row).toBe(2);
    expect(listKey(1, 3, "Home").row).toBe(0);
    expect(listKey(1, 3, "Enter")).toEqual({ row: 1, activate: true });
    expect(listKey(-1, 3, "Enter").activate).toBe(false);
    expect(listKey(0, 0, "Enter").activate).toBe(false);
    expect(listKey(1, 3, "a")).toEqual({ row: 1, activate: false });
    expect(listKey(0, 0, "ArrowDown").row).toBe(-1);
  });
  it("nextSource walks the origins like a vertical tablist", () => {
    expect(nextSource("local", "ArrowDown")).toBe("clone");
    expect(nextSource("local", "ArrowUp")).toBe(SOURCES[SOURCES.length - 1]);
    expect(nextSource("clone", "End")).toBe("discover");
    expect(nextSource("discover", "Home")).toBe("local");
    expect(nextSource("discover", "ArrowDown")).toBe("local");
    expect(nextSource("github", "x")).toBe("github");
  });
});

describe("remembered choices", () => {
  it("round-trips the last origin and the clone parent, and ignores junk", () => {
    expect(lastSource()).toBe("local");
    rememberSource("github");
    expect(lastSource()).toBe("github");
    localStorage.setItem("axelstra.addProject.tab", "bogus");
    expect(lastSource()).toBe("local");
    rememberParent("  /home/me/code ");
    expect(lastParent()).toBe("/home/me/code");
  });
  it("survives a storage that throws", () => {
    const boom = () => {
      throw new Error("blocked");
    };
    vi.stubGlobal("localStorage", { getItem: boom, setItem: boom });
    expect(lastSource()).toBe("local");
    expect(lastParent()).toBe("");
    expect(() => rememberSource("new")).not.toThrow();
    expect(() => rememberParent("/x")).not.toThrow();
  });
});

describe("looksLikeFolder", () => {
  it("is a hint: no extension means a folder", () => {
    expect(looksLikeFolder("/home/me/proj")).toBe(true);
    expect(looksLikeFolder("C:\\Users\\me\\proj\\")).toBe(true);
    expect(looksLikeFolder("/home/me/notes.txt")).toBe(false);
  });
});
