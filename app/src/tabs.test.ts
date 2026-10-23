import { afterEach, describe, expect, it, vi } from "vitest";
import { createComponent } from "solid-js";
import { render } from "solid-js/web";
import { Tab } from "./Tab";
import { Tabs, TabsList } from "./ui";

describe("tabs", () => {
  let dispose = () => {};
  afterEach(() => {
    dispose();
    document.body.innerHTML = "";
  });

  // the same shape as the App strip: manual activation (arrows move focus, a click or Enter picks), one Tab per value
  const mount = (sel: number, extra: { onRename?: () => void; onClose?: () => void } = {}) => {
    const picked: number[] = [];
    const host = document.createElement("div");
    document.body.append(host);
    dispose = render(
      () =>
        createComponent(Tabs, {
          value: `t${sel}`,
          activationMode: "manual",
          onChange: () => {},
          get children() {
            return createComponent(TabsList, {
              get children() {
                return [0, 1, 2].map((i) => createComponent(Tab, { value: `t${i}`, selected: i === sel, onSelect: () => picked.push(i), ...(i === 1 ? extra : {}), children: `T${i}` }));
              },
            });
          },
        }),
      host,
    );
    return picked;
  };
  const tabEls = () => [...document.querySelectorAll<HTMLElement>('[role="tab"]')];
  const key = (el: Element, k: string) => el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));

  it("has a tablist with a tab per tab, aria-selected and a roving tabindex", () => {
    mount(1);
    expect(document.querySelectorAll('[role="tablist"]').length).toBe(1);
    expect(tabEls().map((t) => t.getAttribute("aria-selected"))).toEqual(["false", "true", "false"]);
    expect(tabEls().map((t) => t.tabIndex)).toEqual([-1, 0, -1]);
  });

  it("never nests a button inside a button", () => {
    mount(0, { onClose: () => {} });
    expect(document.querySelectorAll("button button").length).toBe(0);
    expect(document.querySelectorAll('[role="tab"] button').length).toBe(0);
    expect(document.querySelectorAll('[role="tab"]').length).toBe(3);
    expect(document.querySelector('button[aria-label="Fechar"]')).not.toBeNull();
  });

  it("arrow keys move focus between tabs and wrap, Home and End jump", () => {
    mount(0);
    const tabs = tabEls();
    tabs[0].focus();
    key(tabs[0], "ArrowRight");
    expect(document.activeElement).toBe(tabs[1]);
    key(tabs[1], "End");
    expect(document.activeElement).toBe(tabs[2]);
    key(tabs[2], "ArrowRight");
    expect(document.activeElement).toBe(tabs[0]);
    key(tabs[0], "ArrowLeft");
    expect(document.activeElement).toBe(tabs[2]);
    key(tabs[2], "Home");
    expect(document.activeElement).toBe(tabs[0]);
  });

  it("arrowing only moves focus; a click picks the tab", () => {
    const picked = mount(0);
    const tabs = tabEls();
    tabs[0].focus();
    key(tabs[0], "ArrowRight");
    expect(picked).toEqual([]);
    tabs[1].click();
    expect(picked).toEqual([1]);
  });

  it("arrows on a close button do not move between tabs", () => {
    mount(1, { onClose: () => {} });
    const x = document.querySelector<HTMLElement>('button[aria-label="Fechar"]')!;
    tabEls()[1].focus();
    x.focus();
    expect(document.activeElement).toBe(x);
    key(x, "ArrowRight");
    expect(document.activeElement).toBe(x);
  });

  it("F2 renames the focused tab", () => {
    const onRename = vi.fn();
    mount(1, { onRename });
    key(tabEls()[1], "F2");
    expect(onRename).toHaveBeenCalledTimes(1);
  });

  it("middle click closes a tab that can close", () => {
    const onClose = vi.fn();
    mount(1, { onClose });
    tabEls()[1].dispatchEvent(new MouseEvent("auxclick", { button: 1, bubbles: true, cancelable: true }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
