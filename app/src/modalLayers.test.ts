import { afterEach, describe, expect, it, vi } from "vitest";
import { createComponent } from "solid-js";
import { render } from "solid-js/web";
import { ContextMenu, menuOpen, showMenuAt } from "./ContextMenu";
import { Modal } from "./Modal";

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
});
