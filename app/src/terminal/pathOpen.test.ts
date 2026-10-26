import { describe, expect, it, vi } from "vitest";
import type { PathInfo } from "../api";
import { AppError } from "../appError";
import { choicesFor, classifyPath, describe as describeInfo, openPath, type PathDeps } from "./pathOpen";

const info = (o: Partial<PathInfo> = {}): PathInfo => ({
  path: String.raw`C:\w\proj\a.bin`,
  is_dir: false,
  exec: false,
  inside: true,
  can_open: true,
  can_run: false,
  ...o,
});

const link = (path: string, line?: number) => ({ text: path, start: 0, path, line });

/** deps that never touch the system: every effect is recorded */
const deps = (o: { info?: PathInfo | Error; answer?: string | null; wt?: string } = {}) => {
  const log: string[] = [];
  const d: PathDeps = {
    worktree: () => o.wt ?? "C:/w/proj",
    resolve: vi.fn(async () => {
      if (o.info instanceof Error) throw o.info;
      return o.info ?? info();
    }),
    act: vi.fn(async (_s, p, a) => (log.push(`act ${a} ${p}`), { path: p })),
    openHtml: vi.fn(async (_s, p) => void log.push(`html ${p}`)),
    openEditor: (rel, line) => void log.push(`editor ${rel}:${line ?? ""}`),
    choose: vi.fn(async () => o.answer ?? null),
    copy: async (t) => void log.push(`copy ${t}`),
    warn: (t) => void log.push(`warn ${t}`),
  };
  return { d, log };
};

describe("classifyPath", () => {
  it("sends html to the browser, code and text to the editor, the rest to the dialog", () => {
    expect(classifyPath("out/Index.HTML")).toBe("html");
    expect(classifyPath(String.raw`C:\a b\page.htm`)).toBe("html");
    expect(classifyPath("src/a.ts")).toBe("text");
    expect(classifyPath(String.raw`C:\Users\x y\notes.md`)).toBe("text");
    expect(classifyPath("Makefile")).toBe("text");
    expect(classifyPath(".gitignore")).toBe("text");
    for (const p of [
      String.raw`C:\Users\franc\Desktop\Projetos\axelstra\target\release\bundle\nsis\Axelstra_0.1.0_x64-setup.exe`,
      "x.msi",
      "x.bat",
      "x.cmd",
      "x.ps1",
      "x.sh",
      "x.dll",
      "x.lnk",
      "photo.png",
      "doc.pdf",
      "archive.zip",
      "dir/noext",
      "../up.exe",
    ])
      expect(classifyPath(p), p).toBe("other");
  });
});

describe("choicesFor", () => {
  it("offers run for programs, open for plain files, and always reveal; never both run and open", () => {
    expect(choicesFor(info({ can_open: false, can_run: true })).map((c) => c.id)).toEqual(["run", "reveal"]);
    expect(choicesFor(info()).map((c) => c.id)).toEqual(["open", "reveal"]);
    expect(choicesFor(info({ can_open: false })).map((c) => c.id)).toEqual(["reveal"]);
  });
  it("describes the canonical path, the type and the warning for programs", () => {
    const t = describeInfo(info({ path: "C:\\p q\\s.exe", exec: true, can_run: true, can_open: false }));
    expect(t).toContain("C:\\p q\\s.exe");
    expect(t.split("\n").length).toBeGreaterThan(2);
    expect(describeInfo(info({ inside: false, can_open: false }))).not.toBe(describeInfo(info()));
  });
});

describe("openPath", () => {
  it("opens code inside the worktree in the editor at its line, with no dialog", async () => {
    const { d, log } = deps({ info: info({ path: "C:\\w\\proj\\src\\a.ts" }) });
    await openPath(link(String.raw`C:\w\proj\src\a.ts`, 12), 3, d);
    expect(log).toEqual(["editor src/a.ts:12"]);
    expect(d.choose).not.toHaveBeenCalled();
  });

  it("opens .html in the browser through the host, also with spaces", async () => {
    const { d, log } = deps();
    await openPath(link(String.raw`C:\w\proj\my docs\Index.html`), 3, d);
    expect(log).toEqual([String.raw`html C:\w\proj\my docs\Index.html`]);
  });

  it("warns, visibly, when the path does not exist", async () => {
    const { d, log } = deps({
      info: new AppError("files.resolve_path", "nope", "files.path_not_found", { path: "x.exe" }),
    });
    await openPath(link("x.exe"), 3, d);
    expect(log).toHaveLength(1);
    expect(log[0]).toMatch(/^warn /);
    expect(d.choose).not.toHaveBeenCalled();
  });

  it("asks before running a program, then runs the canonical path only on the run answer", async () => {
    const exe = String.raw`C:\Users\franc\Desktop\Projetos\axelstra\target\release\bundle\nsis\Axelstra_0.1.0_x64-setup.exe`;
    const prog = info({ path: exe, exec: true, can_run: true, can_open: false });
    const yes = deps({ info: prog, answer: "run" });
    await openPath(link(exe), 3, yes.d);
    expect(yes.log).toEqual([`act run ${exe}`]);
    const no = deps({ info: prog, answer: null });
    await openPath(link(exe), 3, no.d);
    expect(no.log).toEqual([]);
    expect(no.d.choose).toHaveBeenCalledTimes(1);
  });

  it("ignores an answer that was not one of the offered buttons (open on a program)", async () => {
    const { d, log } = deps({ info: info({ exec: true, can_run: false, can_open: false }), answer: "open" });
    await openPath(link("a.exe"), 3, d);
    expect(log).toEqual([]);
  });

  it("outside the project, or a folder, can only be revealed", async () => {
    const out = deps({ info: info({ inside: false, can_open: false, can_run: false }), answer: "reveal" });
    await openPath(link(String.raw`D:\other\x.png`), 3, out.d);
    expect(out.log).toEqual([`act reveal ${String.raw`C:\w\proj\a.bin`}`]);
    const dir = deps({ info: info({ is_dir: true, can_open: false }), answer: "reveal" });
    await openPath(link("dist"), 3, dir.d);
    expect(dir.log).toEqual([`act reveal ${String.raw`C:\w\proj\a.bin`}`]);
  });

  it("a text file outside the worktree gets the dialog, not the editor", async () => {
    const { d, log } = deps({ info: info({ inside: false, can_open: false, path: "D:\\o\\n.txt" }), answer: null });
    await openPath(link(String.raw`D:\o\n.txt`), 3, d);
    expect(log).toEqual([]);
    expect(d.choose).toHaveBeenCalledTimes(1);
  });

  it("warns when an action fails", async () => {
    const { d, log } = deps({ answer: "open" });
    d.act = vi.fn(async () => Promise.reject(new Error("boom")));
    await openPath(link("a.bin"), 3, d);
    expect(log.some((l) => l.startsWith("warn"))).toBe(true);
  });

  it("a remote worktree only offers to copy the path", async () => {
    const { d, log } = deps({ wt: "@lab:/home/me/proj", answer: "copy" });
    await openPath(link("/home/me/proj/x.sh"), 3, d);
    expect(d.resolve).not.toHaveBeenCalled();
    expect(log).toEqual(["copy /home/me/proj/x.sh"]);
  });

  it("a terminal without a session answers with a warning", async () => {
    const { d, log } = deps();
    await openPath(link("x.exe"), undefined, d);
    expect(log).toHaveLength(1);
    expect(log[0]).toMatch(/^warn /);
  });
});
