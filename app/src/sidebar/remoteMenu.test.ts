import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => undefined) }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));

import type { MenuItem } from "../ContextMenu";
import { projectMenu, worktreeMenu, type MenuDeps } from "./menus";

const deps = { copy: vi.fn(), run: vi.fn() } as unknown as MenuDeps;
const openIn = (items: MenuItem[]) =>
  items.find(
    (i) =>
      ("label" in i && i.children?.some((c) => "label" in c && c.label === "VS Code")) ||
      ("label" in i && /Abrir em/.test(i.label)),
  );
const wt = (path: string) => ({ path, branch: "main", is_main: true });

describe("remote projects in the sidebar menus", () => {
  it("disables open externally for a remote worktree and copies the remote path", () => {
    const items = worktreeMenu(deps, { id: "p", name: "p" }, wt("@lab:/home/me/p"), "main");
    const open = openIn(items) as { disabled?: boolean; label: string };
    expect(open.disabled).toBe(true);
    expect(open.label).toMatch(/remotos/);
    const copy = items.find((i) => "label" in i && i.icon === "copy") as { run: () => void };
    copy.run();
    expect(deps.copy).toHaveBeenCalledWith("/home/me/p");
  });
  it("keeps open externally for a local project", () => {
    const open = openIn(projectMenu(deps, { id: "p", name: "p", path: "C:/work/p" })) as { disabled?: boolean };
    expect(open.disabled).toBe(false);
    const w = openIn(worktreeMenu(deps, { id: "p", name: "p" }, wt("C:/work/p"), "main")) as { disabled?: boolean };
    expect(w.disabled).toBe(false);
  });
  it("disables it for a remote project too", () => {
    const open = openIn(projectMenu(deps, { id: "p", name: "p", path: "@lab:/home/me/p" })) as { disabled?: boolean };
    expect(open.disabled).toBe(true);
  });
});
