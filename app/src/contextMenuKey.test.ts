import { describe, expect, it } from "vitest";
import { isMenuKey, menuKey, menuOpen } from "./ContextMenu";
import { press } from "./a11y";

describe("keyboard context menus (U2)", () => {
  it("recognises the menu key and Shift+F10 only", () => {
    expect(isMenuKey({ key: "ContextMenu", shiftKey: false })).toBe(true);
    expect(isMenuKey({ key: "F10", shiftKey: true })).toBe(true);
    expect(isMenuKey({ key: "F10", shiftKey: false })).toBe(false);
    expect(isMenuKey({ key: "Enter", shiftKey: false })).toBe(false);
  });
  it("menuKey opens the menu under the row and ignores keys from children", () => {
    const row = document.createElement("div");
    const child = document.createElement("button");
    row.append(child);
    document.body.append(row);
    const h = menuKey(() => [{ label: "Renomear" }]);
    const mk = (target: HTMLElement) => {
      const e = new KeyboardEvent("keydown", { key: "F10", shiftKey: true, cancelable: true });
      Object.defineProperty(e, "target", { value: target });
      Object.defineProperty(e, "currentTarget", { value: row });
      return e;
    };
    expect(h(mk(child) as never)).toBe(false);
    expect(menuOpen()).toBe(false);
    expect(h(mk(row) as never)).toBe(true);
    expect(menuOpen()).toBe(true);
  });
  it("press passes keys to the row handler and still activates on Enter", () => {
    let seen = "";
    let hit = 0;
    const p = press(
      () => hit++,
      "button",
      (e) => (seen = e.key),
    );
    const el = document.createElement("div");
    const e = new KeyboardEvent("keydown", { key: "Enter", cancelable: true });
    Object.defineProperty(e, "target", { value: el });
    Object.defineProperty(e, "currentTarget", { value: el });
    (p.onKeyDown as (e: KeyboardEvent) => void)(e);
    expect(seen).toBe("Enter");
    expect(hit).toBe(1);
  });
});
