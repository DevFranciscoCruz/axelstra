import { afterEach, describe, expect, it, vi } from "vitest";
import { createComponent } from "solid-js";
import { render } from "solid-js/web";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => undefined }));
vi.mock("./api", () => ({ call: async () => ({}), callBg: async () => ({}), listSessions: async () => [], listProjects: async () => [], worktreeStatus: async () => [], ghListBg: async () => [], gitChanges: async () => null, flowTasks: async () => [] }));

import { Modal } from "./Modal";
import { Palette } from "./Palette";

const press = (init: KeyboardEventInit) => window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));

// jsdom has no layout: scrolling a row into view is a no-op there
Element.prototype.scrollIntoView = () => {};

describe("command palette", () => {
  let dispose = () => {};
  afterEach(() => {
    dispose();
    document.body.innerHTML = "";
  });

  const actions = (n: number) => Array.from({ length: n }, (_, i) => ({ label: `Ação ${i}`, run: () => {} }));
  const mount = (n: number, onClose = () => {}) => {
    dispose = render(
      () => [createComponent(Modal, { label: "Por baixo", onClose, get children() { return document.createElement("button"); } }), createComponent(Palette, { actions: () => actions(n) })],
      document.body,
    );
    press({ key: "k", ctrlKey: true });
  };

  it("is a modal dialog", () => {
    mount(3);
    const d = document.querySelector('.pal[role="dialog"]')!;
    expect(d.getAttribute("aria-modal")).toBe("true");
    expect(d.getAttribute("aria-label")).toBe("Paleta de comandos");
  });

  it("lists more than 12 results", () => {
    mount(40);
    expect(document.querySelectorAll(".pal-item").length).toBe(40);
  });

  it("Esc closes only the palette, not the dialog under it", () => {
    const under = vi.fn();
    mount(3, under);
    press({ key: "Escape" });
    expect(document.querySelector(".pal")).toBeNull();
    expect(under).not.toHaveBeenCalled();
    press({ key: "Escape" });
    expect(under).toHaveBeenCalledTimes(1);
  });

  it("Enter with no match stays open and says so", () => {
    mount(3);
    const input = document.querySelector<HTMLInputElement>(".pal-input")!;
    input.value = "zzzz";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(document.querySelector(".pal")).not.toBeNull();
    expect(document.querySelector(".pal-empty")!.textContent).toContain("zzzz");
  });

  it("a row under a still pointer is not selected, a moving pointer selects", () => {
    mount(5);
    const rows = document.querySelectorAll<HTMLElement>(".pal-item");
    rows[3].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true, clientX: 5, clientY: 5 }));
    expect(rows[0].classList.contains("on")).toBe(true);
    rows[3].dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 6, clientY: 6 }));
    expect(rows[3].classList.contains("on")).toBe(true);
  });

  it("ranks by match quality and remembers what was run", () => {
    dispose = render(() => createComponent(Palette, { actions: () => [{ id: "a", label: "Reiniciar daemon", run: () => {} }, { id: "b", label: "Definições", run: () => {} }, { id: "c", label: "Docker", run: () => {} }] }), document.body);
    press({ key: "k", ctrlKey: true });
    const labels = () => [...document.querySelectorAll(".pal-item span:first-child")].map((e) => e.textContent);
    const input = document.querySelector<HTMLInputElement>(".pal-input")!;
    input.value = "d";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    expect(labels()).toEqual(["Docker", "Definições", "Reiniciar daemon"]);
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    press({ key: "k", ctrlKey: true });
    expect(labels()[0]).toBe("Definições");
  });

  it("gives focus back to the dialog it was opened over", () => {
    mount(3);
    const under = document.querySelector('[aria-label="Por baixo"]')!;
    expect(under.contains(document.activeElement)).toBe(false);
    press({ key: "Escape" });
    expect(under.contains(document.activeElement)).toBe(true);
  });
});
