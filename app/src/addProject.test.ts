import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const h = vi.hoisted(() => ({
  picked: 0,
  pickResult: null as null | { id: string; name: string; path: string; worktrees: unknown[] },
  added: [] as string[][],
  cloned: [] as Record<string, unknown>[],
  cancelled: [] as string[],
  status: [] as Record<string, unknown>[],
  gh: { state: "ok", repos: [] } as { state: string; repos: Record<string, unknown>[] },
  discovered: [] as Record<string, unknown>[],
  discAdded: [] as string[],
  inited: [] as Record<string, unknown>[],
  folder: "/home/me/code" as string | null,
  info: { wsl_distros: [] as string[], ssh_hosts: [] as string[] },
}));

vi.mock("@tauri-apps/api/core", () => ({ Channel: class {}, invoke: async () => undefined }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => undefined }),
}));
vi.mock("./tauriEvents", () => ({ onTauriEvent: () => () => undefined }));
vi.mock("./api", async (orig) => ({
  ...(await orig<typeof import("./api")>()),
  hostsList: async () => [{ id: "lab", name: "Lab", spec: "ssh:lab", slot: 1, state: "ok" }],
  hostInfo: async () => ({
    os: "windows",
    home: "",
    user: "",
    hostname: "",
    shell: "",
    sep: "\\",
    proto: 1,
    build: "",
    ...h.info,
  }),
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
    return h.pickResult;
  },
  pickFolder: async () => h.folder,
  projectClone: async (p: Record<string, unknown>) => {
    h.cloned.push(p);
    return { id: "c1" };
  },
  projectCloneStatus: async () => h.status.shift() ?? { state: "running" },
  projectCloneCancel: async (id: string) => {
    h.cancelled.push(id);
    return { ok: true };
  },
  ghRepos: async () => h.gh,
  projectDiscover: async () => h.discovered,
  projectAddDiscovered: async (path: string) => {
    h.discAdded.push(path);
    return { id: "d1", name: "found", path, worktrees: [] };
  },
  projectInit: async (p: Record<string, unknown>) => {
    h.inited.push(p);
    return { id: "n1", name: String(p.name), path: `${p.parent}/${p.name}`, worktrees: [] };
  },
  listProjects: async () => [],
  listSessions: async () => [],
}));

import { AddProject } from "./AddProject";
import { resetClone } from "./addProject/cloneJob";
import { addProjectTab, loadHosts, overlay, setAddProjectTab, setShowAddProject } from "./store";

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

const settle = async () => {
  for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0));
};
const button = (text: string) =>
  [...document.querySelectorAll("button")].find((b) => b.textContent?.includes(text)) as HTMLButtonElement;
const tab = (text: string) =>
  [...document.querySelectorAll<HTMLElement>("[role=tab]")].find((t) => t.textContent?.includes(text))!;
const input = (id: string) => document.getElementById(id) as HTMLInputElement;
const type = (el: HTMLInputElement, v: string) => fireEvent.input(el, { target: { value: v } });
const open = async (tabName: Parameters<typeof setShowAddProject>[1] = "local") => {
  setShowAddProject(true, tabName);
  render(() => createComponent(AddProject, {}));
  await settle();
};

beforeEach(async () => {
  h.picked = 0;
  h.pickResult = null;
  h.added = [];
  h.cloned = [];
  h.cancelled = [];
  h.status = [];
  h.gh = { state: "ok", repos: [] };
  h.discovered = [];
  h.discAdded = [];
  h.inited = [];
  h.folder = "/home/me/code";
  h.info = { wsl_distros: [], ssh_hosts: [] };
  vi.stubGlobal("localStorage", memory());
  await loadHosts();
});
afterEach(() => {
  cleanup();
  resetClone();
  vi.useRealTimers();
  setShowAddProject(false);
});

describe("adicionar projeto: o diálogo", () => {
  it("is one accessible dialog with a vertical list of the six origins", async () => {
    await open();
    const dialog = document.querySelector("[role=dialog]")!;
    expect(dialog.getAttribute("aria-label")).toBe("Adicionar projeto");
    const list = document.querySelector("[role=tablist]")!;
    expect(list.getAttribute("aria-orientation")).toBe("vertical");
    expect(list.getAttribute("aria-label")).toBe("De onde vem o projeto?");
    expect([...document.querySelectorAll("[role=tab]")].map((t) => t.querySelector("span")?.textContent)).toEqual([
      "Pasta local",
      "Clonar repositório",
      "GitHub",
      "Novo projeto",
      "Remoto",
      "Descobertos",
    ]);
    expect(tab("Pasta local").getAttribute("aria-selected")).toBe("true");
  });

  it("arrows move and select, Home/End jump, and the choice is remembered", async () => {
    await open();
    tab("Pasta local").focus();
    fireEvent.keyDown(tab("Pasta local"), { key: "ArrowDown" });
    await settle();
    expect(addProjectTab()).toBe("clone");
    fireEvent.keyDown(tab("Clonar repositório"), { key: "End" });
    await settle();
    expect(addProjectTab()).toBe("discover");
    fireEvent.keyDown(tab("Descobertos"), { key: "Home" });
    await settle();
    expect(addProjectTab()).toBe("local");
    expect(localStorage.getItem("axelstra.addProject.tab")).toBe("local");
  });

  it("opens at the origin used last time", async () => {
    localStorage.setItem("axelstra.addProject.tab", "new");
    await open("choose");
    expect(tab("Novo projeto").getAttribute("aria-selected")).toBe("true");
    expect(document.getElementById("ap-new-parent")).toBeTruthy();
  });

  it("Esc and Close shut it without adding anything", async () => {
    await open();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(overlay()).toBeNull();
    setShowAddProject(true);
    cleanup();
    render(() => createComponent(AddProject, {}));
    fireEvent.click(button("Fechar"));
    expect(overlay()).toBeNull();
    expect(h.added).toEqual([]);
    expect(h.picked).toBe(0);
  });
});

describe("pasta local", () => {
  it("opens the native picker and keeps the dialog when cancelled", async () => {
    await open();
    fireEvent.click(button("Escolher pasta"));
    await settle();
    expect(h.picked).toBe(1);
    expect(overlay()).not.toBeNull();
  });

  it("closes the dialog once a folder was added", async () => {
    h.pickResult = { id: "p1", name: "p", path: "/p", worktrees: [] };
    await open();
    fireEvent.click(button("Escolher pasta"));
    await settle();
    expect(overlay()).toBeNull();
  });

  it("Enter on the list of origins runs the main action of the panel", async () => {
    await open();
    fireEvent.keyDown(tab("Pasta local"), { key: "Enter" });
    await settle();
    expect(h.picked).toBe(1);
  });
});

describe("remoto", () => {
  it("shows the remote form as a tab and submits it", async () => {
    await open("remote");
    expect(document.querySelector("input[placeholder='/home/utilizador/projeto']")).toBeTruthy();
    expect(button("Voltar")).toBeUndefined();
    fireEvent.click(button("Adicionar projeto"));
    await settle();
    expect(h.added).toEqual([["lab", "/home/me"]]);
    expect(overlay()).toBeNull();
  });

  it("offers detected WSL distros and ssh hosts, and fills the new host form", async () => {
    h.info = { wsl_distros: ["Ubuntu"], ssh_hosts: ["lab", "dev"] };
    await open("remote");
    // `lab` is already a host: not suggested again
    expect(button("lab") === undefined || !button("lab").getAttribute("aria-label")?.includes("Usar")).toBe(true);
    fireEvent.click(document.querySelector<HTMLElement>("[aria-label='Usar wsl:Ubuntu']")!);
    await settle();
    const spec = document.querySelector<HTMLInputElement>(
      "input[placeholder='ssh:utilizador@servidor ou wsl:Ubuntu']",
    )!;
    expect(spec.value).toBe("wsl:Ubuntu");
    expect(document.querySelector("[aria-label='Usar ssh:dev']")).toBeTruthy();
    expect(document.querySelector("[aria-label='Usar ssh:lab']")).toBeNull();
  });
});

describe("clonar repositório", () => {
  it("validates, suggests the folder name, and starts the clone with the options", async () => {
    await open("clone");
    fireEvent.click(document.querySelector<HTMLElement>("[data-ap-primary]")!);
    await settle();
    expect(document.getElementById("ap-clone-url-msg")!.textContent).toContain("Escreve o endereço");
    expect(h.cloned).toEqual([]);

    type(
      document.querySelector<HTMLInputElement>("input[placeholder^='https://github.com']")!,
      "https://github.com/o/repo.git",
    );
    await settle();
    type(input("ap-clone-parent"), "/home/me/code");
    const inputs = [...document.querySelectorAll<HTMLInputElement>("form input")];
    expect(inputs.find((i) => i.value === "repo")).toBeTruthy();
    type(document.querySelector<HTMLInputElement>("input[placeholder='o ramo por omissão']")!, "dev");
    type(document.querySelector<HTMLInputElement>("input[placeholder='tudo']")!, "1");
    fireEvent.click(document.querySelector<HTMLElement>("[data-ap-primary]")!);
    await settle();
    expect(h.cloned).toEqual([
      { url: "https://github.com/o/repo.git", dest: "/home/me/code/repo", branch: "dev", depth: 1 },
    ]);
    expect(localStorage.getItem("axelstra.addProject.cloneParent")).toBe("/home/me/code");
  });

  it("refuses an option-looking address and a bad depth", async () => {
    await open("clone");
    type(document.querySelector<HTMLInputElement>("input[placeholder^='https://github.com']")!, "--upload-pack=x");
    fireEvent.click(document.querySelector<HTMLElement>("[data-ap-primary]")!);
    await settle();
    expect(document.getElementById("ap-clone-url-msg")!.textContent).toContain("«-»");
    expect(h.cloned).toEqual([]);
  });

  it("the folder button fills the field from the native picker", async () => {
    await open("clone");
    fireEvent.click(button("Escolher…"));
    await settle();
    expect(input("ap-clone-parent").value).toBe("/home/me/code");
  });

  it("follows the progress, closes on success, and cancel reaches the daemon", async () => {
    await open("clone");
    type(
      document.querySelector<HTMLInputElement>("input[placeholder^='https://github.com']")!,
      "https://github.com/o/repo.git",
    );
    type(input("ap-clone-parent"), "/w");
    vi.useFakeTimers();
    fireEvent.click(document.querySelector<HTMLElement>("[data-ap-primary]")!);
    await vi.advanceTimersByTimeAsync(0);
    h.status = [{ state: "running", percent: 40, phase: "Receiving objects" }];
    await vi.advanceTimersByTimeAsync(500);
    expect(document.querySelector("progress")?.getAttribute("value")).toBe("40");
    expect(document.body.textContent).toContain("Receiving objects");
    fireEvent.click(button("Cancelar clonagem"));
    await vi.advanceTimersByTimeAsync(0);
    expect(h.cancelled).toEqual(["c1"]);
    h.status = [{ state: "cancelled" }];
    await vi.advanceTimersByTimeAsync(500);
    expect(document.body.textContent).toContain("Clonagem cancelada.");
    vi.useRealTimers();
  });

  it("a failed clone shows the reason in an alert and keeps the form", async () => {
    await open("clone");
    type(
      document.querySelector<HTMLInputElement>("input[placeholder^='https://github.com']")!,
      "https://github.com/o/repo.git",
    );
    type(input("ap-clone-parent"), "/w");
    vi.useFakeTimers();
    fireEvent.click(document.querySelector<HTMLElement>("[data-ap-primary]")!);
    await vi.advanceTimersByTimeAsync(0);
    h.status = [{ state: "failed", error: "fatal: repository not found" }];
    await vi.advanceTimersByTimeAsync(500);
    const alerts = [...document.querySelectorAll("[role=alert]")].map((a) => a.textContent);
    expect(alerts.some((t) => t?.includes("A clonagem falhou: fatal: repository not found"))).toBe(true);
    expect(overlay()).not.toBeNull();
    vi.useRealTimers();
  });
});

describe("GitHub", () => {
  const repos = [
    {
      name_with_owner: "me/alpha",
      description: "A tool",
      is_private: false,
      url: "https://github.com/me/alpha.git",
      pushed_at: null,
    },
    {
      name_with_owner: "me/beta",
      description: null,
      is_private: true,
      url: "https://github.com/me/beta.git",
      pushed_at: null,
    },
  ];

  it("hints at `gh auth login` when gh is not signed in, and at the install when it is missing", async () => {
    h.gh = { state: "not_authed", repos: [] };
    await open("github");
    expect(document.body.textContent).toContain("gh auth login");
    cleanup();
    h.gh = { state: "no_gh", repos: [] };
    await open("github");
    expect(document.body.textContent).toContain("cli.github.com");
  });

  it("filters, moves with the arrows and Enter clones into the chosen folder", async () => {
    h.gh = { state: "ok", repos };
    await open("github");
    type(input("ap-gh-parent"), "/home/me/code");
    expect(document.querySelectorAll("[role=option]").length).toBe(2);
    const filter = document.querySelector<HTMLInputElement>("input[role=combobox]")!;
    type(filter, "BET");
    await settle();
    expect(document.querySelectorAll("[role=option]").length).toBe(1);
    fireEvent.keyDown(filter, { key: "ArrowDown" });
    await settle();
    expect(filter.getAttribute("aria-activedescendant")).toBe("ap-gh-list-0");
    fireEvent.keyDown(filter, { key: "Enter" });
    await settle();
    expect(h.cloned).toEqual([{ url: "https://github.com/me/beta.git", dest: "/home/me/code/beta" }]);
  });

  it("a click clones; without a folder it asks for one first", async () => {
    h.gh = { state: "ok", repos };
    await open("github");
    fireEvent.click(document.querySelectorAll("[role=option]")[0]);
    await settle();
    expect(h.cloned).toEqual([]);
    expect(document.getElementById("ap-gh-parent-msg")!.textContent).toContain("Escolhe a pasta");
    type(input("ap-gh-parent"), "/w");
    fireEvent.click(document.querySelectorAll("[role=option]")[0]);
    await settle();
    expect(h.cloned).toEqual([{ url: "https://github.com/me/alpha.git", dest: "/w/alpha" }]);
  });
});

describe("novo projeto", () => {
  it("checks the name, then creates the project with the chosen extras", async () => {
    await open("new");
    type(input("ap-new-parent"), "/home/me/code");
    const name = document.querySelector<HTMLInputElement>("input[placeholder='o-meu-projeto']")!;
    type(name, "a/b");
    fireEvent.click(document.querySelector<HTMLElement>("[data-ap-primary]")!);
    await settle();
    expect(document.getElementById("ap-new-name-msg")!.textContent).toContain("uma só pasta");
    expect(h.inited).toEqual([]);
    type(name, "novo");
    fireEvent.click(document.querySelector<HTMLElement>("[data-ap-primary]")!);
    await settle();
    expect(h.inited).toEqual([{ parent: "/home/me/code", name: "novo", readme: true, gitignore: false, commit: true }]);
    expect(overlay()).toBeNull();
  });
});

describe("descobertos", () => {
  it("lists repositories and Enter adds the highlighted one", async () => {
    h.discovered = [
      { path: "/home/me/code/one", name: "one", branch: "main", source: "scan" },
      { path: "/home/me/code/two", name: "two", source: "recent" },
    ];
    await open("discover");
    expect(document.querySelectorAll("[role=option]").length).toBe(2);
    const list = document.getElementById("ap-ds-list")!;
    fireEvent.keyDown(list, { key: "ArrowDown" });
    fireEvent.keyDown(list, { key: "ArrowDown" });
    fireEvent.keyDown(list, { key: "Enter" });
    await settle();
    expect(h.discAdded).toEqual(["/home/me/code/two"]);
    expect(overlay()).toBeNull();
  });

  it("says so when there is nothing to add", async () => {
    await open("discover");
    expect(document.body.textContent).toContain("Não encontrei repositórios novos");
  });
});

describe("store", () => {
  it("the tab can be changed while open", async () => {
    await open();
    setAddProjectTab("clone");
    await settle();
    expect(tab("Clonar repositório").getAttribute("aria-selected")).toBe("true");
  });
});
