import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createComponent } from "solid-js";
import type { WorkspacePreview } from "./workspaceLogic";
import { WS } from "./strings.workspace";

const api = vi.hoisted(() => ({
  pickAndExportWorkspace: vi.fn(),
  pickWorkspaceFile: vi.fn(),
  mapWorkspaceFolder: vi.fn(),
  clearWorkspaceMap: vi.fn(),
  applyWorkspaceImport: vi.fn(),
  cancelWorkspaceImport: vi.fn(() => Promise.resolve()),
}));
vi.mock("./api", () => api);
const store = vi.hoisted(() => ({ setError: vi.fn(), showNotice: vi.fn() }));
vi.mock("./store", () => store);

const sec = (o = {}) => ({ new: [], present: [], conflict: [], skipped: [], local_only: 0, ...o });
const pv = (o: Partial<WorkspacePreview> = {}): WorkspacePreview => ({
  version: 1,
  exported_at: "2026-10-24T10:00:00Z",
  projects: sec({
    new: ["a (/a)"],
    skipped: [{ label: "b (/old/b)", reason: "missing", path: "/old/b" }],
    local_only: 2,
  }),
  automations: sec(),
  settings: sec({ conflict: ["terse"] }),
  cards: sec({ new: ["a/one"] }),
  risky_cards: 1,
  map: [],
  file: "ws.json",
  ...o,
});

beforeEach(() => [...Object.values(api), ...Object.values(store)].forEach((f) => f.mockClear()));
afterEach(() => cleanup());

const open = async (preview = pv()) => {
  api.pickWorkspaceFile.mockResolvedValueOnce(preview);
  const { WorkspaceSection } = await import("./WorkspaceSection");
  const r = render(() => createComponent(WorkspaceSection, {}));
  fireEvent.click(r.getByRole("button", { name: WS.import }));
  await waitFor(() => expect(document.body.textContent).toContain("ws.json"));
  return r;
};

describe("WorkspaceSection", () => {
  it("export: a cancelled dialog says nothing, a result is shown in the toast", async () => {
    const { WorkspaceSection } = await import("./WorkspaceSection");
    const r = render(() => createComponent(WorkspaceSection, {}));
    api.pickAndExportWorkspace.mockResolvedValueOnce(undefined);
    fireEvent.click(r.getByRole("button", { name: WS.export }));
    await waitFor(() => expect(api.pickAndExportWorkspace).toHaveBeenCalledTimes(1));
    expect(store.showNotice).not.toHaveBeenCalled();
    await waitFor(() => expect((r.getByRole("button", { name: WS.export }) as HTMLButtonElement).disabled).toBe(false));
    api.pickAndExportWorkspace.mockResolvedValueOnce({
      path: "/x/ws.json",
      summary: { projects: 1, automations: 0, cards: 0 },
    });
    fireEvent.click(r.getByRole("button", { name: WS.export }));
    await waitFor(() => expect(store.showNotice).toHaveBeenCalledWith(WS.exported("/x/ws.json")));
  });

  it("import: shows the preview with counts, skipped reasons and the card warning before anything is written", async () => {
    await open();
    const text = document.body.textContent ?? "";
    expect(text).toContain("1 novo");
    expect(text).toContain(WS.reasons.missing);
    expect(text).toContain(WS.risky(1));
    expect(api.applyWorkspaceImport).not.toHaveBeenCalled();
  });

  it("merge applies at once with keep; the dialog closes and the toast says what happened", async () => {
    api.applyWorkspaceImport.mockResolvedValueOnce({
      projects_added: 1,
      projects_updated: 0,
      automations_added: 0,
      automations_updated: 0,
      settings_set: 0,
      cards_written: 1,
      cards_kept: 0,
      files: ["projects.json"],
    });
    await open();
    fireEvent.click(document.body.querySelector("button.primary") as HTMLElement);
    await waitFor(() => expect(api.applyWorkspaceImport).toHaveBeenCalledWith("merge", "keep"));
    await waitFor(() => expect(store.showNotice).toHaveBeenCalled());
    expect(store.showNotice.mock.calls[0][0]).toContain(WS.doneTitle);
    await waitFor(() => expect(document.body.textContent).not.toContain("ws.json"));
  });

  it("replace stays disabled until the word is typed", async () => {
    api.applyWorkspaceImport.mockResolvedValueOnce({
      projects_added: 0,
      projects_updated: 0,
      automations_added: 0,
      automations_updated: 0,
      settings_set: 0,
      cards_written: 0,
      cards_kept: 0,
      files: [],
    });
    await open();
    fireEvent.click(document.body.querySelector(`input[type="radio"][value="replace"]`) as HTMLElement);
    const ok = document.body.querySelector("button.primary") as HTMLButtonElement;
    await waitFor(() => expect(ok.disabled).toBe(true));
    fireEvent.click(ok);
    expect(api.applyWorkspaceImport).not.toHaveBeenCalled();
    const input = document.body.querySelector(`input[type="text"], input:not([type])`) as HTMLInputElement;
    fireEvent.input(input, { target: { value: WS.replaceWord } });
    await waitFor(() => expect(ok.disabled).toBe(false));
    fireEvent.click(ok);
    await waitFor(() => expect(api.applyWorkspaceImport).toHaveBeenCalledWith("replace", "keep"));
  });

  it("cancel forgets the picked file on the host", async () => {
    await open();
    fireEvent.click(
      [...document.body.querySelectorAll("button")].find((b) => b.textContent === WS.cancel) as HTMLElement,
    );
    expect(api.cancelWorkspaceImport).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(document.body.textContent).not.toContain("ws.json"));
  });

  it("a skipped project offers the native folder pick and shows the new preview", async () => {
    api.mapWorkspaceFolder.mockResolvedValueOnce(
      pv({ file: "ws.json", projects: sec({ new: ["a", "b"] }), map: [["/old/b", "/new/b"]] }),
    );
    await open();
    fireEvent.click(
      [...document.body.querySelectorAll("button")].find((b) => b.textContent === WS.mapButton) as HTMLElement,
    );
    await waitFor(() => expect(api.mapWorkspaceFolder).toHaveBeenCalledWith(WS.mapPick, "/old/b"));
    await waitFor(() => expect(document.body.textContent).toContain("/old/b → /new/b"));
  });

  it("an old daemon is told so", async () => {
    api.pickWorkspaceFile.mockResolvedValueOnce(null);
    const { WorkspaceSection } = await import("./WorkspaceSection");
    const r = render(() => createComponent(WorkspaceSection, {}));
    fireEvent.click(r.getByRole("button", { name: WS.import }));
    await waitFor(() => expect(store.showNotice).toHaveBeenCalledWith(WS.noSupport));
  });
});
