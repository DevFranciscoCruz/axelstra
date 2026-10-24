import { beforeEach, describe, expect, it, vi } from "vitest";
import type { HostInfo } from "./api";

const h = (over: Partial<HostInfo> = {}): HostInfo => ({
  id: "lab",
  name: "Lab",
  spec: "ssh:lab",
  slot: 1,
  state: "idle",
  ...over,
});
const api = vi.hoisted(() => ({
  list: [] as unknown[],
  reconnect: async (_id: string): Promise<unknown> => ({}),
  events: [] as ((p: unknown) => void)[],
}));

vi.mock("./api", () => ({
  hostsList: async () => api.list,
  hostAdd: async (name: string, spec: string) => ({ id: name.toLowerCase(), name, spec, slot: 2, state: "connecting" }),
  hostRemove: async () => undefined,
  hostReconnect: (id: string) => api.reconnect(id),
}));
vi.mock("./tauriEvents", () => ({
  onTauriEvent: (_n: string, cb: (p: unknown) => void) => {
    api.events.push(cb);
    return () => undefined;
  },
}));
const refresh = vi.hoisted(() => vi.fn(async () => undefined));
vi.mock("./store/git", () => ({ refreshProjects: refresh }));

const load = async () => {
  vi.resetModules();
  return import("./store/hosts");
};

describe("hosts store", () => {
  beforeEach(() => {
    api.list = [h()];
    api.events = [];
    refresh.mockClear();
  });

  it("loads the list and follows host-state events", async () => {
    const s = await load();
    const stop = s.watchHosts();
    await vi.waitFor(() => expect(s.hosts()).toHaveLength(1));
    api.events[0]({ id: "lab", state: "connecting" });
    expect(s.hostState("lab")).toBe("connecting");
    api.events[0]({ id: "lab", state: "ok" });
    expect(s.hostState("lab")).toBe("ok");
    expect(refresh).toHaveBeenCalledTimes(1);
    api.events[0]({ id: "lab", state: "lost", error: "ssh exited" });
    expect(s.hostById("lab")?.error).toBe("ssh exited");
    stop();
  });

  it("reconnect shows connecting, then the answer", async () => {
    const s = await load();
    await s.loadHosts();
    api.reconnect = async () => {
      expect(s.hostState("lab")).toBe("connecting");
      return h({ state: "ok", proto: 18 });
    };
    await s.reconnectHost("lab");
    expect(s.hostById("lab")).toMatchObject({ state: "ok", proto: 18 });
  });

  it("a failed reconnect leaves the host lost with the reason", async () => {
    const s = await load();
    await s.loadHosts();
    api.reconnect = async () => Promise.reject(new Error("Permission denied (publickey)"));
    await expect(s.reconnectHost("lab")).rejects.toThrow();
    expect(s.hostById("lab")).toMatchObject({ state: "lost", error: "Permission denied (publickey)" });
  });

  it("add and remove update the list", async () => {
    const s = await load();
    await s.loadHosts();
    await s.addHost(" Wsl1 ", " wsl:Ubuntu ");
    expect(s.hosts().map((x) => x.id)).toEqual(["lab", "wsl1"]);
    expect(s.hostName("wsl1")).toBe("Wsl1");
    expect(s.hostName("gone")).toBe("gone");
    await s.removeHost("lab");
    expect(s.hosts().map((x) => x.id)).toEqual(["wsl1"]);
  });
});
