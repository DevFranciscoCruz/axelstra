import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createComponent } from "solid-js";
import { SettingsPage } from "./SettingsPage";
import { SECTION_KEY, SECTIONS, setSection, visibleItems, loadSection, effectiveSection, section } from "./settingsNav";
import { activate, activeSession } from "./store/sessions";
import { overlay, setOverlay, setShowSettings, showSettings } from "./store/overlay";
import { L } from "./strings.logic";

const caps = { hibernate: true, build: true };
const T = L.settingsNav;

const mount = (onClose: () => void = vi.fn(), c = caps) =>
  render(() => createComponent(SettingsPage, { caps: c, onClose, page: (id) => `page:${id}` }));

const memory = (): Storage => {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
    clear: () => m.clear(),
    key: (i: number) => [...m.keys()][i] ?? null,
    get length() {
      return m.size;
    },
  };
};

beforeEach(() => {
  // newer Node ships its own (unusable) localStorage that shadows jsdom's: use a plain in-memory one
  vi.stubGlobal("localStorage", memory());
  setSection("general");
  setOverlay(null);
});
afterEach(() => {
  cleanup();
  document.body.innerHTML = "";
});

describe("settings navigation logic", () => {
  it("every page has texts and the list is searchable without accents", () => {
    expect(visibleItems(caps, "").map((i) => i.id)).toEqual(SECTIONS.map((s) => s.id));
    expect(visibleItems(caps, "memoria").map((i) => i.id)).toContain("memory");
    expect(visibleItems(caps, "zzzzzz")).toEqual([]);
  });
  it("pages of settings an old daemon lacks are not listed", () => {
    const ids = visibleItems({ hibernate: false, build: false }, "").map((i) => i.id);
    expect(ids).not.toContain("hibernate");
    expect(ids).not.toContain("build");
    expect(effectiveSection("build", { hibernate: false, build: false })).toBe("general");
  });
  it("the saved page is validated and a broken storage gives the first", () => {
    expect(loadSection(() => "memory")).toBe("memory");
    expect(loadSection(() => "nope")).toBe("general");
    expect(loadSection(() => null)).toBe("general");
  });
  it("picking a page remembers it", () => {
    setSection("about");
    expect(localStorage.getItem(SECTION_KEY)).toBe("about");
    expect(section()).toBe("about");
  });
});

describe("SettingsPage", () => {
  it("lists the sections in a navigation and marks the current one", () => {
    const r = mount();
    const nav = r.getByRole("navigation", { name: T.navLabel });
    expect(nav).toBeTruthy();
    const cur = nav.querySelectorAll('[aria-current="page"]');
    expect(cur).toHaveLength(1);
    expect(cur[0].textContent).toContain(T.items.general.title);
    expect(r.container.textContent).toContain("page:general");
  });
  it("clicking a section shows it, once, and the choice survives a reopen", () => {
    const r = mount();
    fireEvent.click(r.getByRole("button", { name: T.items.notifications.title }));
    expect(r.container.textContent).toContain("page:notifications");
    expect(r.container.textContent).not.toContain("page:general");
    expect(r.getByRole("heading", { level: 2 }).textContent).toBe(T.items.notifications.title);
    cleanup();
    const again = mount();
    expect(again.container.textContent).toContain("page:notifications");
  });
  it("search narrows the list", () => {
    const r = mount();
    fireEvent.input(r.getByRole("searchbox", { name: T.search }), { target: { value: "ssh" } });
    const nav = r.getByRole("navigation");
    expect(nav.textContent).toContain(T.items.connections.title);
    expect(nav.textContent).not.toContain(T.items.about.title);
    fireEvent.input(r.getByRole("searchbox"), { target: { value: "zzzz" } });
    expect(r.container.textContent).toContain(T.noMatch);
  });
  it("Esc leaves; Esc first clears a search", () => {
    const onClose = vi.fn();
    const r = mount(onClose);
    const box = r.getByRole("searchbox") as HTMLInputElement;
    fireEvent.input(box, { target: { value: "ssh" } });
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
    expect(box.value).toBe("");
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
  it("Esc belongs to a dialog that is open above the page", () => {
    const onClose = vi.fn();
    mount(onClose);
    document.body.insertAdjacentHTML("beforeend", '<div role="dialog"></div>');
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
  });
  it("the back button leaves", () => {
    const onClose = vi.fn();
    const r = mount(onClose);
    fireEvent.click(r.getByRole("button", { name: new RegExp(T.back) }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
  it("the section list is not a modal: no dialog role, nothing traps Tab", () => {
    const r = mount();
    expect(r.container.querySelector('[role="dialog"], [aria-modal]')).toBeNull();
  });
  it("opening and leaving the page keeps the active session", () => {
    activate(7, "user");
    setShowSettings(true);
    expect(showSettings()).toBe(true);
    mount(() => setShowSettings(false));
    fireEvent.keyDown(document, { key: "Escape" });
    expect(overlay()).toBeNull();
    expect(activeSession()).toBe(7);
  });
});
