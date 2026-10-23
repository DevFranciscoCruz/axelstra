import { describe, expect, it } from "vitest";
import { LEFT, MIN_CENTER, RIGHT, fitPanel, resizeTo, splitTo } from "./layout";

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
    expect(fitPanel(10, RIGHT, 0, 3000)).toBe(240);
  });
});

describe("resizeTo", () => {
  it("moves with the arrows, in the direction the panel grows", () => {
    expect(resizeTo(260, "ArrowRight", false, 1, LEFT)).toBe(276);
    expect(resizeTo(260, "ArrowLeft", false, 1, LEFT)).toBe(244);
    expect(resizeTo(320, "ArrowLeft", false, -1, RIGHT)).toBe(336);
    expect(resizeTo(320, "ArrowRight", true, -1, RIGHT)).toBe(256);
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
