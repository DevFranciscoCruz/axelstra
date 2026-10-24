import { beforeEach, describe, expect, it } from "vitest";
import { overlay, setOverlay, setShowProjectSettings, setShowSettings, setShowShortcuts, showProjectSettings, showSettings, showShortcuts } from "./store/overlay";

beforeEach(() => setOverlay(null));

describe("one overlay signal", () => {
  it("opens one dialog at a time", () => {
    setShowSettings(true);
    setShowShortcuts(true);
    expect(showSettings()).toBe(false);
    expect(showShortcuts()).toBe(true);
    expect(overlay()).toEqual({ kind: "shortcuts" });
  });
  it("closing a dialog that is not the one showing does nothing", () => {
    setShowSettings(true);
    setShowShortcuts(false);
    expect(showSettings()).toBe(true);
    setShowSettings(false);
    expect(overlay()).toBeNull();
  });
  it("project settings carry their project", () => {
    setShowProjectSettings("p1");
    expect(showProjectSettings()).toBe("p1");
    setShowProjectSettings(null);
    expect(showProjectSettings()).toBeNull();
  });
});
