import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { createEffect, on, onCleanup, onMount } from "solid-js";
import {
  agentsList,
  computerStatus,
  budgetEvents,
  janitorEvents,
  listPorts,
  onDaemonEvent,
  onSessionExited,
  restoreSessions,
  stopWatch,
  storeNotices,
  triggersEvents,
  watchWorktree,
} from "./api";
import { errorText } from "./appError";
import { handleBudgetEvents } from "./budgetEvents";
import { handleJanitorEvents } from "./janitorEvents";
import { handleTriggerEvents } from "./triggerEvents";
import { wtName } from "./Usage";
import { S } from "./strings";
import { onTauriEvent } from "./tauriEvents";
import {
  handleSessionExited,
  norm,
  openSession,
  armAutoPick,
  ports,
  refreshChanges,
  refreshProjects,
  refreshSessions,
  retryDaemon,
  selectedWorktree,
  sessionsOf,
  setComputerOn,
  setDaemonReconnecting,
  setError,
  setFsTick,
  setInstalledAgents,
  setPorts,
  showNotice,
  toast,
  watchHosts,
  watchHibernate,
  watchMemory,
} from "./store";
import { L } from "./strings.logic";

/** Everything the shell starts when it opens: startup checks, session restore, pollers, daemon events, the worktree watch and the dev-server port lookup. */
export function useBootstrap() {
  onMount(() => {
    // typing lag diagnosis: stalls of the webview's main thread go to lag.log (see lag_log in the host)
    try {
      const obs = new PerformanceObserver((list) => {
        for (const e of list.getEntries())
          if (e.duration >= 80)
            void tauriInvoke("lag_log", { what: "main thread stalled", ms: Math.round(e.duration) }).catch((err) =>
              console.debug("lag_log", err),
            );
      });
      obs.observe({ entryTypes: ["longtask"] });
      onCleanup(() => obs.disconnect());
    } catch (e) {
      console.debug("longtask observer not supported", e);
    }
  });

  onMount(() => onCleanup(watchHosts()));
  onMount(() => onCleanup(watchMemory()));
  onMount(() => onCleanup(watchHibernate()));

  onMount(
    () =>
      void computerStatus()
        .then(setComputerOn)
        .catch((e) => console.debug("computer.status", e)),
  );

  // notes the daemon kept from its startup (a corrupt state file it recovered from, ...); an older daemon has none
  onMount(
    () =>
      void storeNotices()
        .then((ns) => ns.forEach((text) => toast({ text, kind: "warn", key: `store:${text}` })))
        .catch((e) => console.debug("notices.take", e)),
  );

  // a missing git, or no agent CLI at all, is explained once at startup instead of surfacing as a cryptic failure later
  onMount(
    () =>
      void tauriInvoke<string[]>("missing_tools")
        .then((m) => {
          void agentsList()
            .then((l) => setInstalledAgents(l.filter((a) => a.installed).map((a) => a.id)))
            .catch(() => setInstalledAgents(null));
          const bits: string[] = [];
          if (m.includes("git")) bits.push(S.app.noGit);
          if (m.includes("claude") && m.includes("codex") && m.includes("gemini")) bits.push(S.app.noAgents);
          if (bits.length) showNotice(S.app.missing(bits));
        })
        .catch(() => setInstalledAgents(null)),
  );

  onMount(() => {
    void refreshProjects();
    // sessions of a previous daemon run come back before the first list, so the sidebar never shows them missing
    const restore = (prefix: string) =>
      restoreSessions()
        .catch((e) => {
          setError(S.app.restoreFailed + errorText(e));
          return null;
        })
        .then((r) => {
          void refreshSessions();
          if (!r) return;
          const bits: string[] = [];
          if (r.restored) bits.push(L.restore.restored(r.restored));
          if (r.skipped) bits.push(L.restore.skipped(r.skipped));
          if (r.failed.length) bits.push(S.app.restoreFailedList(r.failed));
          if (bits.length) showNotice(`${prefix}: ${bits.join(" · ")}.`);
        });
    void restore(S.app.onReopen);
    // the daemon crashed or was replaced while the app was open: its sessions are gone, bring them back
    const unlisten = [
      onTauriEvent("daemon-restarted", () => {
        showNotice(S.app.daemonRestarted);
        armAutoPick();
        void restore(S.app.afterRestart);
      }),
      // the host reconnects to the daemon on its own: show it meanwhile, and say so when it did not work
      onTauriEvent("daemon-reconnecting", () => setDaemonReconnecting(true)),
      onTauriEvent<{ ok: boolean } | undefined>("daemon-reconnected", (p) => {
        setDaemonReconnecting(false);
        if (!p?.ok)
          toast({
            text: S.app.reconnectFailed,
            kind: "error",
            key: "daemon-reconnect",
            action: { label: S.toast.retryNow, run: () => void retryDaemon() },
          });
      }),
      // clicking a desktop notification lands on the session that raised it
      onTauriEvent<{ worktree: string; session: number }>("notification-click", (p) => {
        void refreshSessions().then(() => openSession(p.worktree, p.session));
      }),
    ];
    // No polling: the daemon pushes `session.changed`, `fs.changed`, `projects.changed` and the events below on the
    // watch stream (see watchWorktree), and the stream catches up when it reconnects. The change list and the project
    // list also refresh when the window regains focus and after every action the user takes in the sidebar.
    // worktrees the daemon's automatic sweep removed: tell the user and drop them from the sidebar
    const takeJanitor = () =>
      janitorEvents()
        .then((ev) => (handleJanitorEvents(ev) > 0 ? refreshProjects() : undefined))
        .catch((e) => console.debug("janitor.events", e));
    // what the CI-fix trigger did (started a fix task, or held back by a limit)
    const takeTriggers = () =>
      triggersEvents()
        .then(handleTriggerEvents)
        .catch((e) => console.debug("triggers.events", e));
    // token budgets the daemon saw cross 80% or 100%: one toast and one bell entry each
    const takeBudget = () =>
      budgetEvents()
        .then((ev) => handleBudgetEvents(ev, wtName))
        .catch((e) => console.debug("budget.events", e));
    void takeJanitor();
    void takeTriggers();
    void takeBudget();
    const offEvents = [
      onSessionExited(handleSessionExited),
      onDaemonEvent("janitor.events", () => void takeJanitor()),
      onDaemonEvent("triggers.events", () => void takeTriggers()),
      onDaemonEvent("budget.events", () => void takeBudget()),
    ];
    const onFocus = () => {
      void refreshChanges();
      void refreshProjects();
    };
    window.addEventListener("focus", onFocus);
    onCleanup(() => {
      offEvents.forEach((off) => off());
      stopWatch();
      window.removeEventListener("focus", onFocus);
      unlisten.forEach((u) => u());
    });
  });

  // the daemon watches the selected worktree and tells us when files or git state changed
  createEffect(() => {
    const w = selectedWorktree();
    void watchWorktree(
      w,
      (changed) => {
        if (norm(changed) === norm(selectedWorktree())) {
          setFsTick((n) => n + 1);
          void refreshChanges();
        }
      },
      // a dev server that starts or stops prints, and printing then going quiet is a session change: no port polling
      () => void refreshSessions().then(() => refreshPorts()),
      () => void refreshProjects(),
    );
  });

  // dev servers started in this worktree's terminals; a process lookup, so only while something runs and the window is seen.
  // A server that starts or stops prints, and printing then going quiet is a session change: no polling.
  let portsBusy = false;
  const refreshPorts = async () => {
    const w = selectedWorktree();
    if (!w || document.hidden || !sessionsOf(w).some((s) => s.running)) {
      if (ports().length) setPorts([]);
      return;
    }
    // session events come in bursts: one lookup at a time is enough
    if (portsBusy) return;
    portsBusy = true;
    try {
      const p = (await listPorts(w)).map((x) => x.port);
      if (p.join() !== ports().join()) setPorts(p);
    } catch (e) {
      console.debug("ports.list", e);
    } finally {
      portsBusy = false;
    }
  };
  onMount(() => {
    // coming back into view catches up on what changed while hidden
    const vis = () => void refreshPorts();
    document.addEventListener("visibilitychange", vis);
    onCleanup(() => document.removeEventListener("visibilitychange", vis));
  });
  createEffect(
    on(selectedWorktree, () => {
      setPorts([]);
      void refreshPorts();
    }),
  );
}
