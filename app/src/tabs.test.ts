import { afterEach, describe, expect, it, vi } from "vitest";
import { createComponent } from "solid-js";
import { render } from "solid-js/web";
import { tabKeys } from "./a11y";
import { Tab } from "./Tab";

describe("tabs", () => {
  let dispose = () => {};
  afterEach(() => {
    dispose();
    document.body.innerHTML = "";
  });

  const mount = (sel: number, extra: { onRename?: () => void; onClose?: () => void } = {}) => {
    const picked: number[] = [];
    const list = document.createElement("div");
    list.setAttribute("role", "tablist");
    list.addEventListener("keydown", (e) => tabKeys(e as KeyboardEvent & { currentTarget: HTMLElement }));
    document.body.append(list);
    dispose = render(() => [0, 1, 2].map((i) => createComponent(Tab, { selected: i === sel, onSelect: () => picked.push(i), ...(i === 1 ? extra : {}), children: `T${i}` })), list);
    return picked;
  };

  it("has a tab button per tab with aria-selected and a roving tabindex", () => {
    mount(1);
    const tabs = [...document.querySelectorAll<HTMLElement>('[role="tab"]')];
    expect(tabs.map((t) => t.getAttribute("aria-selected"))).toEqual(["false", "true", "false"]);
    expect(tabs.map((t) => t.tabIndex)).toEqual([-1, 0, -1]);
  });

  it("never nests a button inside a button", () => {
    mount(0, { onClose: () => {} });
    expect(document.querySelectorAll("button button").length).toBe(0);
    expect(document.querySelectorAll('[role="tab"] button').length).toBe(0);
  });

  it("arrow keys move focus between tabs and wrap", () => {
    mount(0);
    const tabs = [...document.querySelectorAll<HTMLElement>('[role="tab"]')];
    tabs[0].focus();
    tabs[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(tabs[1]);
    tabs[1].dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(tabs[2]);
    tabs[2].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(tabs[0]);
  });

  it("F2 renames the focused tab", () => {
    const onRename = vi.fn();
    mount(1, { onRename });
    const t = document.querySelectorAll<HTMLElement>('[role="tab"]')[1];
    t.dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true, cancelable: true }));
    expect(onRename).toHaveBeenCalledTimes(1);
  });
});
