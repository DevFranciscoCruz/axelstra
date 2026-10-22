import { describe, expect, it } from "vitest";
import { isTopModal, modalDepth, popModal, pushModal } from "./modalStack";

describe("modal stack", () => {
  it("only the newest modal is on top", () => {
    const a = pushModal();
    const b = pushModal();
    expect(isTopModal(a)).toBe(false);
    expect(isTopModal(b)).toBe(true);
    popModal(b);
    expect(isTopModal(a)).toBe(true);
    popModal(a);
    expect(modalDepth()).toBe(0);
  });
  it("closing a lower modal keeps the top one", () => {
    const a = pushModal();
    const b = pushModal();
    popModal(a);
    expect(isTopModal(b)).toBe(true);
    popModal(b);
  });
  it("popping twice is harmless", () => {
    const a = pushModal();
    popModal(a);
    popModal(a);
    expect(modalDepth()).toBe(0);
  });
});
