import { afterEach, describe, expect, it, vi } from "vitest";
import { createComponent } from "solid-js";
import { render } from "solid-js/web";
import { ContextMenu, menuOpen, showMenuAt } from "./ContextMenu";
import { Modal } from "./Modal";
import { Select } from "./Select";

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
