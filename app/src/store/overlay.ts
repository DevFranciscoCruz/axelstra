import { createSignal } from "solid-js";

/** The one full dialog open from the shell at a time (settings, cheat sheet, ...); null = none. Opening one replaces the other. */
export type Overlay =
  | { kind: "settings" }
  | { kind: "shortcuts" }
  | { kind: "agentSearch" }
  | { kind: "orchestra" }
  | { kind: "dispatcher" }
  | { kind: "automations" }
  | { kind: "addRemote" }
  | { kind: "projectSettings"; project: string };

type Kind = Overlay["kind"];

export const [overlay, setOverlay] = createSignal<Overlay | null>(null);

const isOpen = (k: Kind) => overlay()?.kind === k;
/** closing a dialog only does anything while it is the one showing */
const set = (k: Exclude<Kind, "projectSettings">) => (on: boolean) =>
  setOverlay((cur) => (on ? ({ kind: k } as Overlay) : cur?.kind === k ? null : cur));

export const showSettings = () => isOpen("settings");
export const setShowSettings = set("settings");
export const showShortcuts = () => isOpen("shortcuts");
export const setShowShortcuts = set("shortcuts");
export const showAgentSearch = () => isOpen("agentSearch");
export const setShowAgentSearch = set("agentSearch");
export const showOrchestra = () => isOpen("orchestra");
export const setShowOrchestra = set("orchestra");
export const showDispatcher = () => isOpen("dispatcher");
export const setShowDispatcher = set("dispatcher");
export const showAutomations = () => isOpen("automations");
export const setShowAutomations = set("automations");
export const showAddRemote = () => isOpen("addRemote");
export const setShowAddRemote = set("addRemote");

/** project whose settings dialog (worktree env files, setup command) is open */
export const showProjectSettings = (): string | null => {
  const o = overlay();
  return o?.kind === "projectSettings" ? o.project : null;
};
export const setShowProjectSettings = (id: string | null) =>
  setOverlay((cur) => (id ? { kind: "projectSettings", project: id } : cur?.kind === "projectSettings" ? null : cur));
