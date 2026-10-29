import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createComponent } from "solid-js";

const h = vi.hoisted(() => ({
  diag: {} as Record<string, unknown>,
  calls: 0,
  subs: new Map<string, Set<() => void>>(),
  emit: (name: string) => h.subs.get(name)?.forEach((f) => f()),
}));

vi.mock("./api", () => ({
  diagnostics: vi.fn(async () => {
    h.calls++;
    return { ...h.diag };
  }),
}));
vi.mock("./tauriEvents", () => ({
  onTauriEvent: (name: string, fn: () => void) => {
    if (!h.subs.has(name)) h.subs.set(name, new Set());
    h.subs.get(name)!.add(fn);
    return () => void h.subs.get(name)!.delete(fn);
  },
}));
vi.mock("./daemonActions", () => ({ restartDaemonFlow: vi.fn(async () => true) }));

import { VersionSkew } from "./VersionSkew";
import { restartDaemonFlow } from "./daemonActions";
import { S } from "./strings";

const same = {
  app_version: "0.1.40",
  app_commit: "abcdef1234567",
  app_proto: 32,
  daemon_connected: true,
  daemon_proto: 32,
  daemon_build: "0.1.40+axelstra-daemon",
  daemon_version: "0.1.40",
  daemon_commit: "1234567abc",
  daemon_security: 3,
  app_security_min: 3,
  daemon_offer: null,
};

beforeEach(() => {
  h.diag = { ...same };
  h.calls = 0;
  h.subs.clear();
  vi.mocked(restartDaemonFlow).mockClear();
});
afterEach(() => cleanup());

const mount = () => render(() => createComponent(VersionSkew, {}));

describe("VersionSkew", () => {
  it("is hidden when app and daemon match", async () => {
    const r = mount();
    await waitFor(() => expect(h.calls).toBe(1));
    expect(r.queryByTestId("version-skew")).toBeNull();
  });

  it("shows on daemon-stale with both builds and the reason, no inline style", async () => {
    const r = mount();
    await waitFor(() => expect(h.calls).toBe(1));
    h.diag = { ...same, daemon_offer: { reason: "busy" } };
    h.emit("daemon-stale");
    const bar = await r.findByTestId("version-skew");
    expect(bar.textContent).toContain(S.versionSkew.text("0.1.40 (abcdef1)", "0.1.40 (1234567)"));
    expect(bar.textContent).toContain(S.versionSkew.busy);
    expect(bar.querySelector("[style]")).toBeNull();
  });

  it("re-checks on restarted and reconnected, and hides once the daemon is new", async () => {
    h.diag = { ...same, daemon_build: "0.1.39+x", daemon_version: "0.1.39", daemon_commit: null };
    const r = mount();
    const bar = await r.findByTestId("version-skew");
    expect(bar.textContent).toContain(S.versionSkew.text("0.1.40 (abcdef1)", "0.1.39"));
    h.diag = { ...same };
    h.emit("daemon-restarted");
    await waitFor(() => expect(r.queryByTestId("version-skew")).toBeNull());
    h.diag = { ...same, daemon_offer: { reason: "dev" } };
    h.emit("daemon-reconnected");
    await r.findByTestId("version-skew");
    expect(h.calls).toBe(3);
  });

  it("missing builds read as unknown", async () => {
    h.diag = { ...same, daemon_build: null, daemon_version: null, daemon_commit: null, app_commit: undefined };
    const r = mount();
    const bar = await r.findByTestId("version-skew");
    expect(bar.textContent).toContain(S.versionSkew.text("0.1.40", S.versionSkew.unknown));
  });

  it("restart uses restartDaemonFlow with the specific confirmation", async () => {
    h.diag = { ...same, daemon_offer: { reason: "busy" } };
    const r = mount();
    await r.findByTestId("version-skew");
    fireEvent.click(r.getByText(S.versionSkew.restart));
    expect(restartDaemonFlow).toHaveBeenCalledWith(expect.any(Function), expect.any(Function), S.versionSkew.confirm);
  });

  it("Mais tarde hides it for the session, even when daemon-stale fires again", async () => {
    h.diag = { ...same, daemon_offer: { reason: "busy" } };
    const r = mount();
    await r.findByTestId("version-skew");
    fireEvent.click(r.getByText(S.update.later));
    expect(r.queryByTestId("version-skew")).toBeNull();
    h.emit("daemon-stale");
    await waitFor(() => expect(h.calls).toBe(2));
    expect(r.queryByTestId("version-skew")).toBeNull();
  });
});
