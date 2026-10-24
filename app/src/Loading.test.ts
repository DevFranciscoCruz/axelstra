import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createComponent } from "solid-js";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { Loading } from "./Loading";

describe("Loading", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });
  const mount = (props: Parameters<typeof Loading>[0] = {}) => render(() => createComponent(Loading, props));
  it("is a busy status region", () => {
    const { getByRole } = mount();
    const el = getByRole("status");
    expect(el.getAttribute("aria-busy")).toBe("true");
    expect(el.textContent).toContain("A carregar");
  });
  it("offers a retry after the timeout", () => {
    const retry = vi.fn();
    const { queryByText, getByText } = mount({ onRetry: retry, timeoutMs: 500 });
    expect(queryByText("Tentar de novo")).toBeNull();
    vi.advanceTimersByTime(600);
    fireEvent.click(getByText("Tentar de novo"));
    expect(retry).toHaveBeenCalledTimes(1);
  });
  it("shows no retry button when there is nothing to retry", () => {
    const { queryByText } = mount({ timeoutMs: 100 });
    vi.advanceTimersByTime(200);
    expect(queryByText("Está a demorar mais do que o normal.")).not.toBeNull();
    expect(queryByText("Tentar de novo")).toBeNull();
  });
});
