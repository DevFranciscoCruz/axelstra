import { afterEach, describe, expect, it } from "vitest";
import { installNativeMenuGuard } from "./nativeMenu";

const fire = (el: Element) => {
  const e = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
  el.dispatchEvent(e);
  return e.defaultPrevented;
};
const add = (html: string) => {
  const d = document.createElement("div");
  d.innerHTML = html;
  document.body.append(d);
  return d;
};

describe("native context menu guard", () => {
  let off = () => {};
  afterEach(() => {
    off();
    document.body.innerHTML = "";
    getSelection()?.removeAllRanges();
  });

  it("prod: blocks empty area, agent row and bars", () => {
    off = installNativeMenuGuard(document, false);
    const d = add(`<div id="empty"></div><div class="wt-agent"><button>claude</button></div><div class="bar">x</div>`);
    expect(fire(d.querySelector("#empty")!)).toBe(true);
    expect(fire(d.querySelector("button")!)).toBe(true);
    expect(fire(d.querySelector(".bar")!)).toBe(true);
  });
  it("prod: keeps input, textarea, contenteditable", () => {
    off = installNativeMenuGuard(document, false);
    const d = add(`<input><textarea></textarea><div contenteditable="true"><b>t</b></div>`);
    expect(fire(d.querySelector("input")!)).toBe(false);
    expect(fire(d.querySelector("textarea")!)).toBe(false);
    expect(fire(d.querySelector("b")!)).toBe(false);
  });
  it("prod: keeps the menu over a text selection", () => {
    off = installNativeMenuGuard(document, false);
    const d = add(`<p id="p">some text</p>`);
    const r = document.createRange();
    r.selectNodeContents(d.querySelector("p")!);
    getSelection()!.addRange(r);
    expect(fire(d.querySelector("p")!)).toBe(false);
  });
  it("prod: leaves a component's own handler alone", () => {
    off = installNativeMenuGuard(document, false);
    const d = add(`<div id="x"></div>`);
    let seen = false;
    d.firstElementChild!.addEventListener("contextmenu", () => (seen = true));
    fire(d.firstElementChild!);
    expect(seen).toBe(true);
  });
  it("dev: native menu stays", () => {
    off = installNativeMenuGuard(document, true);
    const d = add(`<div id="empty"></div>`);
    expect(fire(d.firstElementChild!)).toBe(false);
  });
});
