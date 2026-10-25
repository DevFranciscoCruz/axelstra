import { afterEach, describe, expect, it, vi } from "vitest";
import { createComponent } from "solid-js";
import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";

const gitTimeline = vi.hoisted(() => vi.fn());
const setError = vi.hoisted(() => vi.fn());
const showNotice = vi.hoisted(() => vi.fn());

vi.mock("./api", () => ({ gitTimeline }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("./RightTabs", () => ({ RightTabs: () => null }));
vi.mock("./store", () => ({
  findWorktree: () => ({ wt: { path: "/wt" }, project: { id: "p1" } }),
  selectedWorktree: () => "/wt",
  rightTab: () => "timeline",
  setError,
  showNotice,
}));

import { TimelinePanel } from "./TimelinePanel";

const timeline = { branch: "ax/x", base: "main", events: [], tokens: null, ci: null };

describe("TimelinePanel", () => {
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("toggling 'Incluir CI' flips once, reloads once with ci:true and raises no error", async () => {
    gitTimeline.mockResolvedValue(timeline);
    const { getByText, container } = render(() => createComponent(TimelinePanel, {}));
    await waitFor(() => expect(gitTimeline).toHaveBeenCalledTimes(1));
    expect(gitTimeline).toHaveBeenLastCalledWith("/wt", "p1", false);

    const btn = getByText("Incluir CI");
    expect(btn.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(btn);
    expect(btn.getAttribute("aria-pressed")).toBe("true");

    await waitFor(() => expect(gitTimeline).toHaveBeenCalledTimes(2));
    expect(gitTimeline).toHaveBeenLastCalledWith("/wt", "p1", true);
    await new Promise((r) => setTimeout(r, 50));
    expect(gitTimeline).toHaveBeenCalledTimes(2);
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(setError).not.toHaveBeenCalled();
    expect(showNotice).not.toHaveBeenCalled();
  });
});
