import { createSignal } from "solid-js";

/** The one full dialog open from the shell at a time (settings, cheat sheet, ...); null = none. Opening one replaces the other. */
export type Overlay =
  | { kind: "settings" }
  | { kind: "shortcuts" }
  | { kind: "agentSearch" }
  | { kind: "orchestra" }
  | { kind: "dispatcher" }
  | { kind: "automations" }
  | { kind: "addProject"; tab: AddProjectTab }
  | { kind: "projectSettings"; project: string };

/** where the Add project dialog opens: "choose" = the origin used last time, or straight at one of the origins */
export type AddProjectTab = "choose" | "local" | "clone" | "github" | "new" | "remote" | "discover";

type Kind = Overlay["kind"];

export const [overlay, setOverlay] = createSignal<Overlay | null>(null);

const isOpen = (k: Kind) => overlay()?.kind === k;
/** closing a dialog only does anything while it is the one showing */
const set = (k: Exclude<Kind, "projectSettings" | "addProject">) => (on: boolean) =>
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
export const showAddProject = () => isOpen("addProject");
export const addProjectTab = (): AddProjectTab => {
  const o = overlay();
  return o?.kind === "addProject" ? o.tab : "choose";
};
export const setShowAddProject = (on: boolean, tab: AddProjectTab = "choose") =>
  setOverlay((cur) => (on ? { kind: "addProject", tab } : cur?.kind === "addProject" ? null : cur));
/** switch the open Add project dialog between its steps */
export const setAddProjectTab = (tab: AddProjectTab) =>
  setOverlay((cur) => (cur?.kind === "addProject" ? { kind: "addProject", tab } : cur));

/** project whose settings dialog (worktree env files, setup command) is open */
export const showProjectSettings = (): string | null => {
  const o = overlay();
  return o?.kind === "projectSettings" ? o.project : null;
};
export const setShowProjectSettings = (id: string | null) =>
  setOverlay((cur) => (id ? { kind: "projectSettings", project: id } : cur?.kind === "projectSettings" ? null : cur));

/** The part of Settings to scroll to when it opens (the status bar's version chip asks for "about"); consumed once. */
export const [settingsFocus, setSettingsFocus] = createSignal<"about" | null>(null);
/** Opens Settings at the About section. */
export const openAbout = () => {
  setSettingsFocus("about");
  setShowSettings(true);
};
