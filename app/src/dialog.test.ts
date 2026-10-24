import { afterEach, describe, expect, it, vi } from "vitest";
import { createComponent, createSignal } from "solid-js";
import { cleanup, render } from "@solidjs/testing-library";
import { Modal } from "./Modal";
import { modalDepth } from "./modalStack";

const btn = (text: string) => {
  const b = document.createElement("button");
  b.textContent = text;
  return b;
};
const tick = () => new Promise<void>((r) => queueMicrotask(r));
const key = (k: string, init: KeyboardEventInit = {}) => window.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init }));

const shellEl = () => document.querySelector("[data-app-shell]")!;

afterEach(() => {
  cleanup();
  document.body.innerHTML = "";
});

const mountApp = (onClose = () => {}, opts: { dirty?: boolean; closeOnBackdrop?: boolean } = {}) => {
  const [open, setOpen] = createSignal(true);
  const opener = btn("opener");
  const r = render(() => {
    const shell = document.createElement("div");
    shell.setAttribute("data-app-shell", "");
    shell.append(opener);
    return [
      shell,
      () => (open() ? createComponent(Modal, { label: "Teste", onClose, ...opts, get children() { return [btn("a"), btn("b")]; } }) : null),
    ] as never;
  });
  return { ...r, setOpen, opener };
};

describe("Dialog is really modal", () => {
  it("makes the app shell inert while open and restores it on close", async () => {
    const { setOpen } = mountApp();
    await tick();
    expect(shellEl().hasAttribute("inert")).toBe(true);
    expect(document.querySelector('[aria-modal="true"]')).toBeTruthy();
    setOpen(false);
    expect(shellEl().hasAttribute("inert")).toBe(false);
    expect(modalDepth()).toBe(0);
  });

  it("renders in a portal outside the shell", async () => {
    mountApp();
    await tick();
    expect(shellEl().querySelector('[aria-modal="true"]')).toBeNull();
    expect(document.querySelector('[aria-modal="true"]')).toBeTruthy();
  });

  it("moves focus in, traps Tab, and gives focus back on close", async () => {
    const { setOpen } = mountApp();
    await tick();
    // opener was focused before mount; the dialog took focus inside itself
    const [a, b] = [...document.querySelectorAll<HTMLButtonElement>('[aria-modal="true"] button')];
    expect(document.activeElement).toBe(a);
    b.focus();
    key("Tab");
    expect(document.activeElement).toBe(a);
    key("Tab", { shiftKey: true });
    expect(document.activeElement).toBe(b);
    setOpen(false);
    await tick();
  });

  it("backdrop closes, unless dirty or opted out", async () => {
    for (const [opts, closes] of [[{}, true], [{ dirty: true }, false], [{ closeOnBackdrop: false }, false]] as const) {
      const onClose = vi.fn();
      const { unmount } = mountApp(onClose, opts);
      await tick();
      document.querySelector(".modal-back")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      expect(onClose).toHaveBeenCalledTimes(closes ? 1 : 0);
      unmount();
      document.body.innerHTML = "";
    }
  });

  it("a dialog under another one is inert, and the top one wins Esc", async () => {
    const lower = vi.fn();
    const upper = vi.fn();
    const [second, setSecond] = createSignal(false);
    render(() => [
      createComponent(Modal, { label: "Um", onClose: lower, get children() { return btn("x"); } }),
      () => (second() ? createComponent(Modal, { label: "Dois", onClose: upper, get children() { return btn("y"); } }) : null),
    ] as never);
    await tick();
    setSecond(true);
    await tick();
    const backs = document.querySelectorAll(".modal-back");
    expect(backs[0].hasAttribute("inert")).toBe(true);
    expect(backs[1].hasAttribute("inert")).toBe(false);
    key("Escape");
    expect(upper).toHaveBeenCalledTimes(1);
    expect(lower).not.toHaveBeenCalled();
    setSecond(false);
    await tick();
    expect(document.querySelector(".modal-back")!.hasAttribute("inert")).toBe(false);
  });
});
