import { afterEach, describe, expect, it, vi } from "vitest";
import { createComponent } from "solid-js";
import { render } from "solid-js/web";
import { LOCALES, leafPaths, setLocale } from "./i18n";
import { ptP } from "./strings.panels";
import { enP } from "./strings.panels.en";
import { ptBRP } from "./strings.panels.ptbr";

const h = vi.hoisted(() => ({ preview: undefined as unknown, fail: false, calls: 0 }));
vi.mock("./api", () => ({
  removeProjectPreview: vi.fn(async () => {
    h.calls++;
    if (h.fail) throw new Error("boom");
    return h.preview;
  }),
}));

import { RemoveProjectHost, askRemoveProject } from "./RemoveProject";
import { canChoose, doneNotice, effectLines, needsTyped, riskLines } from "./removeProjectLogic";

afterEach(() => setLocale("pt-PT"));

const wt = (branch: string, dirty = false, unmerged = false) => ({ path: `/w/${branch}`, branch, dirty, unmerged });
const prev = (worktrees = [wt("a"), wt("b", true), wt("c", false, true), wt("d", true, true)]) => ({
  sessions: 2,
  terminals: 1,
  worktrees,
});

describe("remove project logic", () => {
  it("lists only worktrees with unsaved or unintegrated work", () => {
    const l = riskLines(prev());
    expect(l).toHaveLength(3);
    expect(l.join("|")).not.toContain("a:");
    expect(l[2]).toContain("d");
  });
  it("no risk without a preview", () => {
    expect(riskLines(null)).toEqual([]);
    expect(riskLines("error")).toEqual([]);
  });
  it("'all' lists sessions, terminals and worktrees; 'list' deletes nothing", () => {
    expect(effectLines("all", prev()).join(" ")).toMatch(/2 .*1 terminal/);
    expect(effectLines("all", { sessions: 0, terminals: 0, worktrees: [] })[0]).toMatch(/Não há/);
    expect(effectLines("list", null).join(" ")).toContain("Nada é apagado");
  });
  it("'all' with risk needs the typed name; only 'list' is open without a preview", () => {
    expect(needsTyped("all", prev())).toBe(true);
    expect(needsTyped("all", prev([wt("a")]))).toBe(false);
    expect(needsTyped("list", prev())).toBe(false);
    expect(canChoose("all", "error")).toBe(false);
    expect(canChoose("all", null)).toBe(false);
    expect(canChoose("list", "error")).toBe(true);
    expect(canChoose("all", prev())).toBe(true);
  });
  it("the notice carries the counts for 'all' and stays plain for 'list'", () => {
    const r = { removed: true, sessions_closed: 3, worktrees_removed: 2, worktrees_pending: 1, branches_deleted: 2 };
    expect(doneNotice("p", "list", r)).toBe("Projeto tirado da lista: p");
    const t = doneNotice("p", "all", r);
    expect(t).toContain("3 sessões fechadas");
    expect(t).toContain("2 worktrees removidos");
    expect(t).toContain("1 worktree ainda por remover");
    expect(doneNotice("p", "all", { ...r, worktrees_pending: 0 })).not.toContain("ainda");
  });
});

describe("remove project dialog", () => {
  let dispose = () => {};
  afterEach(() => {
    dispose();
    document.body.innerHTML = "";
    h.fail = false;
    h.calls = 0;
  });
  const open = async () => {
    dispose = render(() => createComponent(RemoveProjectHost, {}), document.body);
    const p = askRemoveProject("id1", "meu");
    await new Promise((r) => setTimeout(r, 0));
    return p;
  };
  const ok = () => document.querySelector<HTMLButtonElement>(".primary")!;
  const radio = (v: string) => document.querySelector<HTMLInputElement>(`input[type="radio"][value="${v}"]`)!;

  it("defaults to 'list' and reads the preview once", async () => {
    h.preview = prev();
    const p = open();
    await new Promise((r) => setTimeout(r, 0));
    expect(h.calls).toBe(1);
    expect(radio("list").checked).toBe(true);
    expect(document.querySelector(".rm-risk")).not.toBeNull(); // risks visible in red even before choosing
    ok().click();
    expect(await p).toBe("list");
  });
  it("'all' with unsaved work stays disabled until the project name is typed", async () => {
    h.preview = prev();
    const p = open();
    await new Promise((r) => setTimeout(r, 0));
    radio("all").click();
    await new Promise((r) => setTimeout(r, 0));
    expect(ok().disabled).toBe(true);
    const input = document.querySelector<HTMLInputElement>("input[placeholder='meu']")!;
    input.value = "meu";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 0));
    expect(ok().disabled).toBe(false);
    ok().click();
    expect(await p).toBe("all");
  });
  it("cancel resolves null; a failed preview keeps 'all' closed", async () => {
    h.fail = true;
    const p = open();
    await new Promise((r) => setTimeout(r, 0));
    radio("all").click();
    await new Promise((r) => setTimeout(r, 0));
    expect(radio("list").checked).toBe(true);
    document.querySelectorAll<HTMLButtonElement>(".m-actions button")[0].click();
    expect(await p).toBeNull();
  });
});

describe("removeProject texts", () => {
  it("exist in every language with the same keys", () => {
    const keys = leafPaths(ptP.removeProject).sort();
    expect(keys.length).toBeGreaterThan(10);
    expect(leafPaths(enP.removeProject).sort()).toEqual(keys);
    expect(leafPaths(ptBRP.removeProject).sort()).toEqual(keys);
    expect(LOCALES.length).toBe(3);
  });
});
