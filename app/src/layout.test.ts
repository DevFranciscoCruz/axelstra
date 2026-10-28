import { describe, expect, it } from "vitest";
import {
  LEFT,
  MIN_CENTER,
  RIGHT,
  capLeft,
  clampSplit,
  fitPanel,
  resizeTo,
  splitBounds,
  splitTo,
  tabMoveTarget,
} from "./layout";

describe("fitPanel", () => {
  it("keeps a wide window as the user set it", () => {
    expect(fitPanel(300, LEFT, 320, 1600)).toBe(300);
  });
  it("shrinks a panel so the centre keeps its minimum", () => {
    const left = fitPanel(480, LEFT, 320, 900);
    expect(900 - left - 320).toBeGreaterThanOrEqual(MIN_CENTER);
    expect(left).toBe(220);
  });
  it("never goes below the floor in a tiny window", () => {
    expect(fitPanel(260, LEFT, 320, 500)).toBe(120);
  });
  it("respects its own range", () => {
    expect(fitPanel(900, RIGHT, 0, 3000)).toBe(640);
    expect(fitPanel(10, RIGHT, 0, 3000)).toBe(300);
  });
});

describe("resizeTo", () => {
  it("moves with the arrows, in the direction the panel grows", () => {
    expect(resizeTo(260, "ArrowRight", false, 1, LEFT)).toBe(276);
    expect(resizeTo(260, "ArrowLeft", false, 1, LEFT)).toBe(244);
    expect(resizeTo(320, "ArrowLeft", false, -1, RIGHT)).toBe(336);
    expect(resizeTo(320, "ArrowRight", true, -1, RIGHT)).toBe(300);
  });
  it("stops at the ends and jumps with Home/End", () => {
    expect(resizeTo(480, "ArrowRight", true, 1, LEFT)).toBe(480);
    expect(resizeTo(190, "ArrowLeft", true, 1, LEFT)).toBe(180);
    expect(resizeTo(300, "Home", false, 1, LEFT)).toBe(180);
    expect(resizeTo(300, "End", false, 1, LEFT)).toBe(480);
  });
  it("ignores other keys", () => {
    expect(resizeTo(300, "a", false, 1, LEFT)).toBeNull();
  });
});

describe("splitTo", () => {
  it("grows the right pane with Left and shrinks it with Right, within 20% to 80%", () => {
    expect(splitTo(0.5, "ArrowLeft")).toBe(0.55);
    expect(splitTo(0.5, "ArrowRight")).toBe(0.45);
    expect(splitTo(0.8, "ArrowLeft")).toBe(0.8);
    expect(splitTo(0.2, "ArrowRight")).toBe(0.2);
    expect(splitTo(0.5, "x")).toBeNull();
  });
});

describe("capLeft", () => {
  it("caps the list to 34% of a narrow window and leaves wide windows alone", () => {
    expect(capLeft(480, 1000)).toBe(340);
    expect(capLeft(480, 800)).toBe(272);
    expect(capLeft(200, 800)).toBe(200);
    expect(capLeft(480, 1001)).toBe(480);
  });
});

describe("split bounds", () => {
  it("keeps each pane at least 320px, and 0.5 when two panes do not fit", () => {
    expect(splitBounds(1000).min).toBeCloseTo(0.32);
    expect(splitBounds(1000).max).toBeCloseTo(0.68);
    expect(splitBounds(540)).toEqual({ min: 0.5, max: 0.5 });
    expect(splitBounds(3000).min).toBe(0.2);
    expect(splitBounds(0)).toEqual({ min: 0.2, max: 0.8 });
  });
  it("clamps the share and the keyboard steps to them", () => {
    expect(clampSplit(0.2, 1000)).toBe(0.32);
    expect(clampSplit(0.9, 1000)).toBe(0.68);
    expect(splitTo(0.5, "End", 1000)).toBe(0.32);
    expect(splitTo(0.5, "Home", 1000)).toBe(0.68);
    expect(splitTo(0.5, "ArrowLeft", 540)).toBe(0.5);
  });
});

describe("tabMoveTarget", () => {
  const ids = [1, 2, 3, 4];
  it("moves a tab one place left or right", () => {
    expect(tabMoveTarget(ids, 3, -1)).toBe(2);
    expect(tabMoveTarget(ids, 2, 1)).toBe(4);
    expect(tabMoveTarget(ids, 3, 1)).toBeNull();
  });
  it("refuses at the edges and for unknown tabs", () => {
    expect(tabMoveTarget(ids, 1, -1)).toBeUndefined();
    expect(tabMoveTarget(ids, 4, 1)).toBeUndefined();
    expect(tabMoveTarget(ids, 9, 1)).toBeUndefined();
  });
});
