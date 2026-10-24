import { createSignal } from "solid-js";
import { hostAdd, hostReconnect, hostRemove, hostsList, type HostInfo, type HostState } from "../api";
import { applyHostState } from "../hostLogic";
import { onTauriEvent } from "../tauriEvents";
import { refreshProjects } from "./git";

/** The remote hosts the Tauri host knows, with their connection state (empty until `loadHosts` answers). */
export const [hosts, setHosts] = createSignal<HostInfo[]>([]);

export const hostById = (id: string | null | undefined): HostInfo | undefined =>
  id ? hosts().find((h) => h.id === id) : undefined;
/** the name the user gave the host; the id when it is not (yet) listed */
export const hostName = (id: string): string => hostById(id)?.name ?? id;
export const hostState = (id: string): HostState | undefined => hostById(id)?.state;

export const loadHosts = async (): Promise<void> => {
  setHosts(await hostsList());
};

/** `host-state` events keep the list current; returns the stop function. */
export const watchHosts = (): (() => void) => {
  void loadHosts().catch((e) => console.debug("hosts_list", e));
  return onTauriEvent<{ id: string; state: HostState; error?: string | null }>("host-state", (ev) => {
    const before = hostById(ev.id)?.state;
    setHosts((l) => applyHostState(l, ev));
    // a host that came (back) online brings its projects
    if (ev.state === "ok" && before !== "ok") void refreshProjects().catch((e) => console.debug("refreshProjects", e));
  });
};

const put = (h: HostInfo) =>
  setHosts((l) => (l.some((x) => x.id === h.id) ? l.map((x) => (x.id === h.id ? h : x)) : [...l, h]));

export const addHost = async (name: string, spec: string, bin?: string): Promise<HostInfo> => {
  const h = await hostAdd(name.trim(), spec.trim(), bin);
  put(h);
  return h;
};

export const removeHost = async (id: string): Promise<void> => {
  await hostRemove(id);
  setHosts((l) => l.filter((h) => h.id !== id));
  await refreshProjects().catch((e) => console.debug("refreshProjects", e));
};

/** Voltar a ligar: shows "connecting" at once, then whatever the host answers (events may follow). */
export const reconnectHost = async (id: string): Promise<HostInfo | undefined> => {
  setHosts((l) => applyHostState(l, { id, state: "connecting" }));
  try {
    const h = await hostReconnect(id);
    put(h);
    return h;
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    setHosts((l) => applyHostState(l, { id, state: "lost", error: msg }));
    throw e;
  }
};
