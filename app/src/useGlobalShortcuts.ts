import { onCleanup, onMount } from "solid-js";
import type { SessionInfo } from "./api";
import { isTextTarget, overlayOpen } from "./keyboard";
import { S } from "./strings";
import {
  activeSession,
  openSession,
  sessionState,
  sessions,
  setShowAgentSearch,
  setShowSettings,
  setShowShortcuts,
  showNotice,
  toggleRightPanel,
  toggleSidebar,
} from "./store";
import { toggleVoice } from "./Voice";

export type ShortcutDeps = {
  /** tabs of the selected worktree, in order */
  tabs: () => SessionInfo[];
  showTab: (s: SessionInfo) => void;
  reopenClosed: () => void | Promise<void>;
  /** runs a palette command by id (new agent, terminal, worktree); false when it is not available right now */
  run?: (id: string) => boolean;
};

/**
 * The window-level key handler. Ctrl+B hides the project list, Ctrl+Shift+B the right panel; captured so the terminal
 * never sees them. The ones that move around the page leave dialogs, the editor and text fields alone.
 * Exported apart from the hook so tests can call it.
 */
export function createShortcutHandler({ tabs, showTab, reopenClosed, run }: ShortcutDeps) {
  return (e: KeyboardEvent) => {
    // "?" lists the shortcuts, but only where it is not a character being typed
    if (e.key === "?" && !e.ctrlKey && !e.metaKey && !e.altKey) {
      if (overlayOpen() || isTextTarget(e.target) || (e.target as HTMLElement | null)?.closest?.(".xterm")) return;
      e.preventDefault();
      setShowShortcuts(true);
      return;
    }
    if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
    const k = e.key.toLowerCase();
    if (k === "," && !e.shiftKey) {
      if (overlayOpen()) return;
      e.preventDefault();
      e.stopPropagation();
      setShowSettings(true);
      return;
    }
    if (k === "f" && e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      setShowAgentSearch(true);
      return;
    }
    if (k === "m" && e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      void toggleVoice();
      return;
    }
    if (overlayOpen() || isTextTarget(e.target)) return;
    if (k === "t" && e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      void reopenClosed();
      return;
    }
    // the everyday creations, without the terminal ever seeing them (Ctrl+Shift+` is the same key as VS Code's new terminal)
    if (e.shiftKey && run) {
      const id = k === "n" ? "newWorktree" : k === "a" ? "newAgent" : e.code === "Backquote" ? "newTerminal" : "";
      if (id && run(id)) {
        e.preventDefault();
        e.stopPropagation();
        return;
      }
    }
    // Ctrl+Tab / Ctrl+Shift+Tab cycle the tabs of this worktree, Ctrl+1..9 jump to one, Ctrl+J goes to the next agent waiting for an answer
    if (e.key === "Tab" || (/^[1-9]$/.test(e.key) && !e.shiftKey)) {
      const list = tabs();
      if (!list.length) return;
      const at = list.findIndex((t) => t.id === activeSession());
      const pick =
        e.key === "Tab" ? list[(at + (e.shiftKey ? -1 : 1) + list.length) % list.length] : list[Number(e.key) - 1];
      if (!pick) return;
      e.preventDefault();
      e.stopPropagation();
      showTab(pick);
      return;
    }
    if (k === "j" && !e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      const waiting = sessions().filter(
        (x) => x.meta?.kind === "agent" && x.meta.worktree && sessionState(x) === "waiting",
      );
      if (!waiting.length) return showNotice(S.app.noWaiting);
      const next = waiting.find((x) => x.id > activeSession()!) ?? waiting[0];
      openSession(next.meta!.worktree!, next.id);
      return;
    }
    if (k !== "b") return;
    e.preventDefault();
    e.stopPropagation();
    if (e.shiftKey) toggleRightPanel();
    else toggleSidebar();
  };
}

/** Installs the shortcut handler for the life of the calling component. */
export function useGlobalShortcuts(deps: ShortcutDeps) {
  const onKey = createShortcutHandler(deps);
  onMount(() => {
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => window.removeEventListener("keydown", onKey, true));
  });
}
