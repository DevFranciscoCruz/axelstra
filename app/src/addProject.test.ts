import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const h = vi.hoisted(() => ({ picked: 0, added: [] as string[][] }));

vi.mock("@tauri-apps/api/core", () => ({ Channel: class {}, invoke: async () => undefined }));
vi.mock("./tauriEvents", () => ({ onTauriEvent: () => () => undefined }));
vi.mock("./api", async (orig) => ({
  ...(await orig<typeof import("./api")>()),
  hostsList: async () => [{ id: "lab", name: "Lab", spec: "ssh:lab", slot: 1, state: "ok" }],
  hostBrowse: async (_host: string, path?: string) => ({
    path: path ?? "/home/me",
    parent: "/home",
    home: "/home/me",
    entries: [{ name: "proj", git: true }],
  }),
  projectAddRemote: async (host: string, path: string) => {
    h.added.push([host, path]);
    return { id: "@lab:p", name: "proj", path: `@${host}:${path}`, worktrees: [], host };
  },
  pickAndAddProject: async () => {
    h.picked++;
    return null;
  },
  listProjects: async () => [],
  listSessions: async () => [],
}));

import { AddProject } from "./AddProject";
import { addProjectTab, loadHosts, overlay, setShowAddProject } from "./store";

const settle = async () => {
  for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0));
};
const button = (text: string) =>
  [...document.querySelectorAll("button")].find((b) => b.textContent?.includes(text)) as HTMLButtonElement;

beforeEach(async () => {
  h.picked = 0;
  h.added = [];
  await loadHosts();
});
afterEach(() => {
  cleanup();
  setShowAddProject(false);
});

describe("adicionar projeto", () => {
  it("renders an accessible dialog with the two sources", () => {
    setShowAddProject(true);
    render(() => createComponent(AddProject, {}));
    const dialog = document.querySelector("[role=dialog]")!;
    expect(dialog.getAttribute("aria-label")).toBe("Adicionar projeto");
    expect(button("Pasta local")).toBeTruthy();
    expect(button("Projeto remoto")).toBeTruthy();
  });

  it("local folder closes the dialog and opens the native picker", async () => {
    setShowAddProject(true);
    render(() => createComponent(AddProject, {}));
    fireEvent.click(button("Pasta local"));
    await settle();
    expect(h.picked).toBe(1);
    expect(overlay()).toBeNull();
  });

  it("switches to the remote form and back", async () => {
    setShowAddProject(true);
    render(() => createComponent(AddProject, {}));
    fireEvent.click(button("Projeto remoto"));
    await settle();
    expect(addProjectTab()).toBe("remote");
    expect(document.querySelector("input[placeholder='/home/utilizador/projeto']")).toBeTruthy();
    fireEvent.click(button("Voltar"));
    await settle();
    expect(addProjectTab()).toBe("choose");
    expect(button("Pasta local")).toBeTruthy();
  });

  it("opens straight at the remote tab and submits the remote project", async () => {
    setShowAddProject(true, "remote");
    render(() => createComponent(AddProject, {}));
    await settle();
    fireEvent.click(button("Adicionar projeto"));
    await settle();
    expect(h.added).toEqual([["lab", "/home/me"]]);
    expect(overlay()).toBeNull();
  });

  it("Cancel and Esc close without adding anything", async () => {
    setShowAddProject(true, "remote");
    render(() => createComponent(AddProject, {}));
    await settle();
    fireEvent.click(button("Cancelar"));
    expect(overlay()).toBeNull();
    setShowAddProject(true);
    cleanup();
    render(() => createComponent(AddProject, {}));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(overlay()).toBeNull();
    expect(h.added).toEqual([]);
    expect(h.picked).toBe(0);
  });
});
