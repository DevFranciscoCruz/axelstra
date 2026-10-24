import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a), Channel: class {} }));
const open = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...a: unknown[]) => open(...a) }));

beforeEach(() => {
  invoke.mockReset();
  open.mockReset();
});

describe("paths come from the native pick on the Rust side", () => {
  it("adding a project sends no path from the webview and never uses the JS dialog", async () => {
    const api = await import("./api");
    invoke.mockResolvedValue({ id: "p1", name: "p", path: "/p", worktrees: [] });
    expect((await api.pickAndAddProject("t"))?.id).toBe("p1");
    expect(invoke).toHaveBeenCalledWith("project_add_pick", { title: "t" });
    expect(invoke.mock.calls.some((c) => c[0] === "daemon_call" && c[1]?.method === "project.add")).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });

  it("restoring a backup: cancelled = undefined, restored = the answer, old daemon = null", async () => {
    const api = await import("./api");
    invoke.mockResolvedValueOnce(null);
    expect(await api.pickAndRestoreBackup("t")).toBeUndefined();
    invoke.mockResolvedValueOnce({ restored: ["settings.json"] });
    expect(await api.pickAndRestoreBackup("t")).toEqual({ restored: ["settings.json"] });
    expect(invoke).toHaveBeenLastCalledWith("backup_restore_pick", { title: "t" });
    invoke.mockRejectedValueOnce("unknown method backup.restore");
    expect(await api.pickAndRestoreBackup("t")).toBeNull();
    invoke.mockRejectedValueOnce("boom");
    await expect(api.pickAndRestoreBackup("t")).rejects.toThrow();
  });

  it("Settings no longer asks for a typed path", async () => {
    const src = (await import("./Settings.tsx?raw")).default;
    expect(src).not.toContain("window.prompt");
    expect(src).toContain("pickAndRestoreBackup");
  });
});
