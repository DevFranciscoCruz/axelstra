import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";
import { Welcome } from "./Welcome";
import { W } from "./strings.welcome";

afterEach(() => cleanup());

describe("Welcome", () => {
  it("no project: title and add button", () => {
    const onAddProject = vi.fn();
    const r = render(() => createComponent(Welcome, { onAddProject }));
    expect(r.getByRole("heading").textContent).toBe(W.title);
    fireEvent.click(r.getByRole("button", { name: W.add }));
    expect(onAddProject).toHaveBeenCalledTimes(1);
  });

  it("lists the real palette shortcut and the tool hints", () => {
    const r = render(() => createComponent(Welcome, { onAddProject: vi.fn() }));
    expect(r.container.textContent).toContain("Ctrl+K");
    expect(r.container.textContent).toContain("gh");
  });

  it("project without worktree: rows pick, button makes a new one, no add button", () => {
    const onPick = vi.fn();
    const onNewWorktree = vi.fn();
    const r = render(() =>
      createComponent(Welcome, {
        onAddProject: vi.fn(),
        projectName: "proj",
        worktrees: [
          { path: "/w/main", branch: "main", is_main: true },
          { path: "/w/x", branch: "ax/x", is_main: false },
        ],
        onPick,
        onNewWorktree,
      }),
    );
    expect(r.getByRole("heading").textContent).toBe(W.pickTitle);
    expect(r.queryByRole("button", { name: W.add })).toBeNull();
    fireEvent.click(r.getByRole("button", { name: /ax\/x/ }));
    expect(onPick).toHaveBeenCalledWith("/w/x");
    fireEvent.click(r.getByRole("button", { name: W.newWorktree }));
    expect(onNewWorktree).toHaveBeenCalledTimes(1);
  });
});
