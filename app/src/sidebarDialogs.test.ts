import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createComponent, createSignal } from "solid-js";

vi.mock("./store", () => ({ agentKeys: () => ["claude", "codex"] }));
vi.mock("./Icon", () => ({ hasIcon: () => false, Icon: () => null }));

import { CreateWorktree } from "./sidebar/CreateWorktree";
import { RenameWorktree } from "./sidebar/RenameWorktree";

afterEach(() => cleanup());

describe("CreateWorktree dialog", () => {
  const mount = (name: string, onSubmit = vi.fn(), onClose = vi.fn()) => {
    const [value, setValue] = createSignal(name);
    const r = render(() =>
      createComponent(CreateWorktree, {
        name: "proj",
        get newName() {
          return value();
        },
        setNewName: setValue,
        newAgent: "claude",
        setNewAgent: vi.fn(),
        onClose,
        onSubmit,
      }),
    );
    return { r, onSubmit, onClose };
  };
  const button = (label: string) =>
    [...document.querySelectorAll("button")].find((b) => b.textContent?.includes(label)) as HTMLButtonElement;

  it("create is disabled until a name is typed, then submits", async () => {
    const { onSubmit } = mount("");
    expect(button("Criar worktree").disabled).toBe(true);
    const input = document.querySelector<HTMLInputElement>("input[placeholder^='nome do worktree']")!;
    fireEvent.input(input, { target: { value: "feat-x" } });
    expect(button("Criar worktree").disabled).toBe(false);
    fireEvent.click(button("Criar worktree"));
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("Enter is consumed (no click on the button that gets the focus back) and submits only with a name", () => {
    const { onSubmit } = mount("feat-x");
    const input = document.querySelector<HTMLInputElement>("input[placeholder^='nome do worktree']")!;
    const ev = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    input.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(true);
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("the agent selector shows logo and name, never only the logo", () => {
    mount("x");
    const trigger = document.querySelector<HTMLElement>("[aria-label^='Agente:']")!;
    expect(trigger.textContent).toContain("Claude Code");
    expect(trigger.className).not.toContain("sr-only");
    expect(trigger.querySelector(".sr-only")).toBeNull();
  });

  it("Enter submits only with a name; Cancel closes", () => {
    const { onSubmit, onClose } = mount("");
    const input = document.querySelector<HTMLInputElement>("input[placeholder^='nome do worktree']")!;
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.click(button("Cancelar"));
    expect(onClose).toHaveBeenCalled();
  });
});

describe("RenameWorktree dialog", () => {
  it("confirm stays off while the name is unchanged and shows the error", () => {
    const onSubmit = vi.fn();
    render(() =>
      createComponent(RenameWorktree, {
        current: "old",
        value: "old",
        error: "já existe",
        onInput: vi.fn(),
        onClose: vi.fn(),
        onSubmit,
      }),
    );
    const ok = [...document.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("Mudar o nome"),
    ) as HTMLButtonElement;
    expect(ok.disabled).toBe(true);
    expect(document.querySelector("[role=alert]")?.textContent).toBe("já existe");
  });
});
