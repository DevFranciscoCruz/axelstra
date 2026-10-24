import { afterEach, describe, expect, it } from "vitest";
import { setShowSettings } from "./store/overlay";
import { K, isMac, isTextTarget, overlayOpen } from "./keyboard";

afterEach(() => (document.body.innerHTML = ""));

describe("isTextTarget", () => {
  it("is true for inputs and the code editor, false for the terminal and the page", () => {
    document.body.innerHTML =
      '<input id="i"><div class="cm-editor"><div id="cm" contenteditable="true"></div></div><div class="xterm"><textarea id="t"></textarea></div><div id="d"></div>';
    const el = (id: string) => document.getElementById(id);
    expect(isTextTarget(el("i"))).toBe(true);
    expect(isTextTarget(el("cm"))).toBe(true);
    expect(isTextTarget(el("t"))).toBe(false);
    expect(isTextTarget(el("d"))).toBe(false);
    expect(isTextTarget(null)).toBe(false);
  });
});

describe("overlayOpen", () => {
  it("sees dialogs and the palette", () => {
    expect(overlayOpen()).toBe(false);
    document.body.innerHTML = '<div role="dialog"></div>';
    expect(overlayOpen()).toBe(true);
    document.body.innerHTML = '<div class="pal-back"></div>';
    expect(overlayOpen()).toBe(true);
  });
  it("sees the shell overlay signal even before its dialog is in the DOM", () => {
    document.body.innerHTML = "";
    setShowSettings(true);
    expect(overlayOpen()).toBe(true);
    setShowSettings(false);
    expect(overlayOpen()).toBe(false);
  });
});

describe("K", () => {
  it("names Ctrl as Cmd on a Mac and leaves other platforms alone", () => {
    expect(K("Ctrl+Shift+B", true)).toBe("Cmd+Shift+B");
    expect(K("Guardar (Ctrl+S) · Ctrl+Z", true)).toBe("Guardar (Cmd+S) · Cmd+Z");
    expect(K("Ctrl+Shift+B", false)).toBe("Ctrl+Shift+B");
    expect(K("Controlo", true)).toBe("Controlo");
  });
  it("recognises a Mac from the platform string", () => {
    expect(isMac("MacIntel")).toBe(true);
    expect(isMac("Win32")).toBe(false);
  });
});
