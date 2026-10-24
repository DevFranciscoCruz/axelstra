import type { Project, Worktree } from "./api";
import { Handler, paletteCommands } from "./commands";
import { restartDaemonFlow } from "./daemonActions";
import { K } from "./keyboard";
import { requestNotificationsOpen } from "./notifications";
import { S } from "./strings";
import {
  activeSession,
  guarded,
  projectAgent,
  projects,
  setCreatingWorktree,
  setRemoveRequest,
  setRightTab,
  setShowAgentSearch,
  setShowAutomations,
  setShowDispatcher,
  setShowOrchestra,
  setShowSettings,
  setShowShortcuts,
  setView,
  showRightPanel,
  showSidebar,
  toggleRightPanel,
  toggleSidebar,
} from "./store";
import { APP_THEMES, appTheme, setAppTheme } from "./theme";
import { listening, toggleVoice } from "./Voice";

/** What the palette needs from the App that only it owns. */
export type PaletteDeps = {
  current: () => { project: Project; wt: Worktree } | null;
  newAgent: () => void | Promise<void>;
  newTerminal: () => void | Promise<void>;
  toggleSplit: () => void | Promise<void>;
  reopenClosed: () => void | Promise<void>;
  toggleComputer: () => void | Promise<void>;
  computerOn: () => boolean;
  startRename: (id: number) => void;
  /** the dialogs for a new or removed worktree belong to the sidebar: show it first */
  showSidebarNow: () => void;
};

const showPanel = (tab: "files" | "changes" | "pulls" | "timeline") => {
  setRightTab(tab);
  if (!showRightPanel()) toggleRightPanel();
};
const cycleTheme = () => setAppTheme(APP_THEMES[(APP_THEMES.indexOf(appTheme()) + 1) % APP_THEMES.length]);

/** The palette's commands that can run right now (see commands.ts for the list they come from). */
export function buildPaletteActions(d: PaletteDeps) {
  const c = d.current();
  const h: Partial<Record<string, Handler>> = {
    settings: { run: () => setShowSettings(true) },
    shortcuts: { run: () => setShowShortcuts(true) },
    toggleSidebar: { run: toggleSidebar, label: showSidebar() ? S.palette.hideProjects : S.palette.showProjects },
    toggleRight: { run: toggleRightPanel, label: showRightPanel() ? S.palette.hideRight : S.palette.showRight },
    docker: { run: () => guarded(() => setView({ kind: "docker" })) },
    notifications: { run: requestNotificationsOpen },
    theme: { run: cycleTheme, label: S.palette.theme(S.appThemes[appTheme()]) },
    searchChats: { run: () => setShowAgentSearch(true) },
    voice: { run: () => void toggleVoice(), label: listening() ? S.palette.stopVoice : undefined },
    orchestra: { run: () => setShowOrchestra(true) },
    dispatcher: { run: () => setShowDispatcher(true) },
    automations: { run: () => setShowAutomations(true) },
    computer: {
      run: () => void d.toggleComputer(),
      label: d.computerOn() ? S.palette.computerOff : S.palette.computerOn,
    },
    restartDaemon: { run: () => void restartDaemonFlow() },
  };
  if (projects().length > 0)
    h.newWorktree = {
      run: () => {
        d.showSidebarNow();
        setCreatingWorktree(c?.project.id ?? projects()[0].id);
      },
    };
  if (c) {
    h.kanban = { run: () => guarded(() => setView({ kind: "kanban" })) };
    h.panelFiles = { run: () => showPanel("files") };
    h.panelChanges = { run: () => showPanel("changes") };
    h.panelPulls = { run: () => showPanel("pulls") };
    h.newAgent = { run: () => void d.newAgent(), label: S.app.newAgentOf(projectAgent(c.project.id)) };
    h.newTerminal = { run: () => void d.newTerminal() };
    h.split = { run: () => void d.toggleSplit() };
    h.reopen = { run: () => void d.reopenClosed() };
    const active = activeSession();
    if (active !== null) h.rename = { run: () => d.startRename(active) };
    if (!c.wt.is_main)
      h.removeWorktree = {
        run: () => {
          d.showSidebarNow();
          setRemoveRequest({ project: c.project.id, path: c.wt.path, name: c.wt.branch ?? c.wt.path });
        },
      };
  }
  return paletteCommands(h, K);
}
