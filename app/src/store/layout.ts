import { createSignal } from "solid-js";
import { LEFT, NARROW_BELOW, RIGHT, capLeft, fitPanel } from "../layout";
import { readString, writeString } from "../persist";

const load = readString;
const save = (k: string, v: string) => writeString(k, v); // storage unavailable: the choice is just not remembered

/** left project list and right files/changes panel; remembered between runs */
const flag = (k: string) => load(k) !== "0";
const [leftOn, setLeftOn] = createSignal(flag("axelstra.left"));
const [rightOn, setRightOn] = createSignal(flag("axelstra.right"));
export const showSidebar = leftOn;
export const showRightPanel = rightOn;
export const toggleSidebar = () => {
  setLeftOn(!leftOn());
  save("axelstra.left", leftOn() ? "1" : "0");
};
export const toggleRightPanel = () => {
  setRightOn(!rightOn());
  save("axelstra.right", rightOn() ? "1" : "0");
};

/** panel widths in px (drag handles); remembered, clamped on use */
const num = (k: string, d: number) => {
  const n = Number(load(k));
  return Number.isFinite(n) && n > 0 ? n : d;
};
const [leftRaw, setLeftRaw] = createSignal(num("axelstra.leftW", 260));
const [rightRaw, setRightRaw] = createSignal(num("axelstra.rightW", 320));
// the window size, so the saved widths shrink when the window does (and come back when it grows)
const [winW, setWinW] = createSignal(typeof window === "undefined" ? 1400 : window.innerWidth);
// one update per frame while the window is being dragged: each width change refits every terminal
if (typeof window !== "undefined") {
  let frame = 0;
  window.addEventListener("resize", () => {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      setWinW(window.innerWidth);
    });
  });
}
/** widths in use: what was saved, limited so the centre keeps a usable width */
/** under this width the right panel is hidden by CSS (styles.css, `@media (max-width: 1000px)`): it takes no room */
const RIGHT_HIDDEN_BELOW = NARROW_BELOW;
const rightTakes = () => (rightOn() && winW() > RIGHT_HIDDEN_BELOW ? rightRaw() : 0);
/** the width of the project list as shown (also in a narrow window, where it is capped): grid column, handle and aria all read this */
export const leftW = () => capLeft(fitPanel(leftRaw(), LEFT, rightTakes(), winW()), winW());
export const rightW = () => fitPanel(rightRaw(), RIGHT, leftOn() ? leftW() : 0, winW());
export const setLeftW = (w: number) => {
  const v = fitPanel(w, LEFT, rightTakes(), winW());
  setLeftRaw(v);
  save("axelstra.leftW", String(v));
};
export const setRightW = (w: number) => {
  const v = fitPanel(w, RIGHT, leftOn() ? leftW() : 0, winW());
  setRightRaw(v);
  save("axelstra.rightW", String(v));
};
