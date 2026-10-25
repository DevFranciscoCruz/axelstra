import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const h = vi.hoisted(() => ({
  browsed: [] as (string | undefined)[],
  added: [] as string[][],
  browseFail: null as string | null,
  addFail: null as string | null,
}));

vi.mock("@tauri-apps/api/core", () => ({ Channel: class {}, invoke: async () => undefined }));
vi.mock("./tauriEvents", () => ({ onTauriEvent: () => () => undefined }));
vi.mock("./api", async (orig) => ({
  ...(await orig<typeof import("./api")>()),
  hostsList: async () => [{ id: "lab", name: "Lab", spec: "ssh:lab", slot: 1, state: "ok" }],
  hostBrowse: async (_host: string, path?: string) => {
    h.browsed.push(path);
    if (h.browseFail) throw new Error(h.browseFail);
    const at = path ?? "/home/me";
    return {
      path: at,
      parent: at === "/" ? null : "/home",
      home: "/home/me",
      entries: [
        { name: "proj", git: true },
        { name: "docs", git: false },
      ],
    };
  },
  projectAddRemote: async (host: string, path: string) => {
    if (h.addFail) throw new Error(h.addFail);
    h.added.push([host, path]);
    return { id: "@lab:p", name: "proj", path: `@${host}:${path}`, worktrees: [], host };
  },
  listProjects: async () => [],
  listSessions: async () => [],
}));

import { AddRemoteProject } from "./AddRemoteProject";
import { loadHosts } from "./store";

const settle = async () => {
  for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0));
};
const pathInput = () => document.querySelector<HTMLInputElement>("input[placeholder='/home/utilizador/projeto']")!;
const button = (text: string) =>
  [...document.querySelectorAll("button")].find((b) => b.textContent?.includes(text)) as HTMLButtonElement;

beforeEach(async () => {
  h.browsed = [];
  h.added = [];
  h.browseFail = null;
  h.addFail = null;
  await loadHosts();
});
afterEach(cleanup);

describe("adicionar projeto remoto", () => {
  it("starts at the home folder, Enter on a row opens it, confirm adds the shown path", async () => {
    render(() => createComponent(AddRemoteProject, { onClose: () => undefined }));
    await settle();
    expect(h.browsed).toEqual([undefined]);
    expect(pathInput().value).toBe("/home/me");
    const list = document.querySelector<HTMLElement>("[role=listbox]")!;
    fireEvent.keyDown(list, { key: "ArrowDown" });
    fireEvent.keyDown(list, { key: "Enter" });
    await settle();
    expect(h.browsed.at(-1)).toBe("/home/me/proj");
    expect(pathInput().value).toBe("/home/me/proj");
    fireEvent.click(button("Adicionar projeto"));
    await settle();
    expect(h.added).toEqual([["lab", "/home/me/proj"]]);
  });

  it("refuses an empty or relative path without calling the host", async () => {
    render(() => createComponent(AddRemoteProject, { onClose: () => undefined }));
    await settle();
    fireEvent.input(pathInput(), { target: { value: "proj" } });
    fireEvent.keyDown(pathInput(), { key: "Enter" });
    await settle();
    expect(document.body.textContent).toContain("tem de ser absoluto");
    expect(h.added).toEqual([]);
    expect(h.browsed).toEqual([undefined]);
  });

  it("explains an authentication failure and offers to reconnect", async () => {
    h.browseFail = "Permission denied (publickey)";
    render(() => createComponent(AddRemoteProject, { onClose: () => undefined }));
    await settle();
    const alert = document.querySelector("[role=alert]")!;
    expect(alert.textContent).toContain("nunca pede palavra-passe");
    expect(button("Voltar a ligar")).toBeTruthy();
  });

  it("explains a daemon that is not running there", async () => {
    h.browseFail = "daemon not reachable on this machine; start it there";
    render(() => createComponent(AddRemoteProject, { onClose: () => undefined }));
    await settle();
    expect(document.body.textContent).toContain("axelstra daemon");
  });
});
