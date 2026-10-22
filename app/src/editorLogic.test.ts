import { describe, expect, it } from "vitest";
import { LARGE_FILE_CHARS, clampLine, describeReadError, docDiffers, isLarge, sizeLabel } from "./editorLogic";

describe("describeReadError", () => {
  it("explains binary files", () => {
    expect(describeReadError(new Error("binary file"))).toMatch(/binário/);
  });
  it("explains non-UTF-8 and oversize", () => {
    expect(describeReadError("not valid UTF-8 text")).toMatch(/UTF-8/);
    expect(describeReadError("file is larger than 10 MB")).toMatch(/grande/);
  });
  it("passes other errors through", () => {
    expect(describeReadError("file not found")).toBe("file not found");
  });
});

describe("isLarge / sizeLabel", () => {
  it("threshold is exclusive", () => {
    expect(isLarge(LARGE_FILE_CHARS)).toBe(false);
    expect(isLarge(LARGE_FILE_CHARS + 1)).toBe(true);
  });
  it("formats megabytes with comma", () => {
    expect(sizeLabel(3.5 * 1024 * 1024)).toBe("3,5 MB");
  });
});

describe("docDiffers", () => {
  it("skips toString when lengths differ", () => {
    let called = false;
    expect(docDiffers(3, "ab", () => ((called = true), "abc"))).toBe(true);
    expect(called).toBe(false);
  });
  it("compares text when lengths match", () => {
    expect(docDiffers(2, "ab", () => "ab")).toBe(false);
    expect(docDiffers(2, "ab", () => "ac")).toBe(true);
  });
});

describe("clampLine", () => {
  it("clamps into range", () => {
    expect(clampLine(0, 10)).toBe(1);
    expect(clampLine(12, 10)).toBe(10);
    expect(clampLine(5.7, 10)).toBe(5);
  });
  it("returns null without a usable line", () => {
    expect(clampLine(undefined, 10)).toBeNull();
    expect(clampLine(NaN, 10)).toBeNull();
    expect(clampLine(3, 0)).toBeNull();
  });
});
