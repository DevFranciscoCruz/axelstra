import { afterEach, describe, expect, it } from "vitest";
import { isTextTarget, overlayOpen } from "./keyboard";

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
});
