import { afterEach, describe, expect, it, vi } from "vitest";
import { createComponent } from "solid-js";
import { render } from "solid-js/web";
import { ContextMenu, menuOpen, showMenuAt } from "./ContextMenu";
import { Modal } from "./Modal";
import { Select } from "./ui";

const key = (k: string) => window.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));

describe("layers: the topmost one takes Esc and Tab", () => {
  let dispose = () => {};
  afterEach(() => {
    dispose();
    document.body.innerHTML = "";
  });

  const mount = (onClose: () => void) => {
    dispose = render(
      () => [
        createComponent(Modal, { label: "Teste", onClose, get children() { return document.createElement("button"); } }),
        createComponent(ContextMenu, {}),
      ],
      document.body,
    );
  };

  it("Esc with a menu open closes only the menu", () => {
    const onClose = vi.fn();
    mount(onClose);
    showMenuAt(10, 10, [{ label: "Um", run: () => {} }]);
    expect(menuOpen()).toBe(true);
    key("Escape");
    expect(menuOpen()).toBe(false);
    expect(onClose).not.toHaveBeenCalled();
    key("Escape");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Tab with a menu open closes the menu and leaves the dialog open", () => {
    const onClose = vi.fn();
    mount(onClose);
    showMenuAt(10, 10, [{ label: "Um", run: () => {} }]);
    key("Tab");
    expect(menuOpen()).toBe(false);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("a Select announces a listbox of options and its expanded state", () => {
    dispose = render(() => [createComponent(Select, { value: "b", options: [{ value: "a", label: "A" }, { value: "b", label: "B" }], onChange: () => {} }), createComponent(ContextMenu, {})], document.body);
    const btn = document.querySelector<HTMLButtonElement>(".select-btn")!;
    expect(btn.getAttribute("aria-haspopup")).toBe("listbox");
    expect(btn.getAttribute("aria-expanded")).toBe("false");
    btn.click();
    expect(btn.getAttribute("aria-expanded")).toBe("true");
    expect(document.querySelector('[role="listbox"]')).not.toBeNull();
    const opts = [...document.querySelectorAll('[role="option"]')];
    expect(opts.map((o) => o.getAttribute("aria-selected"))).toEqual(["false", "true"]);
    key("Escape");
    expect(btn.getAttribute("aria-expanded")).toBe("false");
  });
});

describe("context menu on Kobalte", () => {
  let dispose = () => {};
  afterEach(() => {
    dispose();
    document.body.innerHTML = "";
  });
  const tick = () => new Promise((r) => setTimeout(r, 20));
  const kd = (el: Element, k: string) => el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));

  it("announces menu/menuitem, focuses the first entry, moves with arrows and runs the picked one", async () => {
    dispose = render(() => createComponent(ContextMenu, {}), document.body);
    const run = vi.fn();
    showMenuAt(10, 10, [{ label: "Um", run: () => {} }, { label: "Off", disabled: true }, { label: "Dois", run }]);
    await tick();
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
    const items = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')];
    expect(items.length).toBe(3);
    expect(document.activeElement).toBe(items[0]);
    kd(items[0], "ArrowDown");
    await tick();
    expect(document.activeElement).toBe(items[2]);
    kd(items[2], "Enter");
    await tick();
    expect(run).toHaveBeenCalledTimes(1);
    expect(menuOpen()).toBe(false);
  });

  it("opens a submenu with ArrowRight and closes it with ArrowLeft", async () => {
    dispose = render(() => createComponent(ContextMenu, {}), document.body);
    showMenuAt(10, 10, [{ label: "Pai", children: [{ label: "Filho", run: () => {} }] }]);
    await tick();
    const parent = document.querySelector<HTMLElement>('[aria-haspopup="menu"]')!;
    parent.focus();
    kd(parent, "ArrowRight");
    await tick();
    const child = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find((e) => e.textContent === "Filho")!;
    expect(child).toBeDefined();
    expect(document.activeElement).toBe(child);
    kd(child, "ArrowLeft");
    await tick();
    expect(document.activeElement).toBe(parent);
  });

  it("Esc returns focus to where it was", async () => {
    dispose = render(() => [document.createElement("button"), createComponent(ContextMenu, {})], document.body);
    const b = document.body.querySelector("button")!;
    b.focus();
    showMenuAt(10, 10, [{ label: "Um" }]);
    await tick();
    key("Escape");
    await tick();
    expect(document.activeElement).toBe(b);
  });
});
