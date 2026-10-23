import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { Icon, hasIcon } from "./Icon";
import { Select } from "./Select";
import { Button, Field, Input, Tabs, TabsList, cn } from "./ui";
import { For, Show, createEffect, createSignal, on, onCleanup, onMount } from "solid-js";
import { SessionInfo, restoreSessions, listPorts, watchWorktree, defaultShell, launchDefaultAgent, killSession, removeSession, spawnSession } from "./api";
import { ChangesPanel } from "./ChangesPanel";
import { CommitView } from "./CommitView";
import { Editor } from "./Editor";
import { FilesPanel } from "./FilesPanel";
import { DockerView } from "./Docker";
import { Kanban } from "./Kanban";
import { DiffView } from "./DiffView";
import { Sidebar } from "./Sidebar";
import { WorktreeReports } from "./WorktreeReports";
import { Palette } from "./Palette";
import { ContextMenu } from "./ContextMenu";
import { Automations } from "./Automations";
import { Dispatcher } from "./Dispatcher";
import { Settings } from "./Settings";
import { Shortcuts } from "./Shortcuts";
import { LEFT, RIGHT, resizeTo, splitTo } from "./layout";
import { K, isTextTarget, overlayOpen } from "./keyboard";
import { openUrl } from "@tauri-apps/plugin-opener";
import { UpdateToast } from "./UpdateToast";
import { PanelIcon } from "./PanelIcon";
import { CloseConfirm } from "./CloseConfirm";
import { ConfirmHost } from "./Confirm";
import { AgentSearch } from "./AgentSearch";
import { Orchestra } from "./Orchestra";
import { UsageChip } from "./Usage";
import { agentsList, computerEnable, computerStatus } from "./api";
import { ProjectSettings } from "./ProjectSettings";
import { StatusBar } from "./StatusBar";
import { MicIcon, VoiceDraft, listening, toggleVoice, voiceSupported } from "./Voice";
import { ErrorToast } from "./ErrorToast";
import { Modal } from "./Modal";
import { Tab } from "./Tab";
import { pickProject } from "./projectActions";
import { StateDot } from "./StateDot";
import { dotOfSession } from "./agentState";
import { ask } from "./Confirm";
import { PullsPanel } from "./PullsPanel";
import { TerminalView } from "./TerminalView";
import {
  activeSession,
  AGENTS,
  agentKeys,
  setInstalledAgents,
  agentPrompt,
  pickAgent,
  defaultAgent,
  fileLine,
  setDaemonReconnecting,
  projectTerse,
  setDefaultAgent,
  changesMode,
  showAutomations,
  setShowAutomations,
  showDispatcher,
  setShowDispatcher,
  setSessionCloser,
  showSettings,
  showAgentSearch,
  setShowAgentSearch,
  setShowSettings,
  setShowShortcuts,
  showShortcuts,
  showView,
  projectAgent,
  tabLabel,
  renameTab,
  sortedTabs,
  moveTab,
  rememberClosed,
  popClosed,
  showOrchestra,
  setShowOrchestra,
  computerOn,
  setComputerOn,
  ports,
  setPorts,
  splitSession,
  setSplitSession,
  setFsTick,
  projects,
  setCreatingWorktree,
  setRemoveRequest,
  norm,
  findWorktree,
  openDiff,
  openCommit,
  openFile,
  rightTab,
  setOpenCommit,
  setOpenFile,
  setOpenDiff,
  setShowDocker,
  setShowProjectSettings,
  showProjectSettings,
  setShowKanban,
  showDocker,
  showKanban,
  refreshChanges,
  refreshProjects,
  refreshSessions,
  sessions,
  sessionState,
  showNotice,
  openSession,
  selectedWorktree,
  sessionsOf,
  setActiveSession,
  setError,
  showSidebar,
  showRightPanel,
  toggleSidebar,
  toggleRightPanel,
  guarded,
  leftW,
  rightW,
  setLeftW,
  setRightW,
} from "./store";

export function App() {
  const [splitFrac, setSplitFrac] = createSignal(0.5);
  // screen control for agents is a deliberate, per-run permission: asking first, and it ends when the daemon does
  const toggleComputer = async () => {
    try {
      if (computerOn()) {
        setComputerOn(await computerEnable(false));
        return;
      }
      const ok = await ask("Permitir que os agentes vejam o ecrã e controlem o rato e o teclado?\n\nPodem tirar capturas de qualquer janela e clicar ou escrever em teu nome. Fica ligado até o desligar aqui ou fechar a aplicação.", {
        title: "Controlo do ecrã",
        kind: "warning",
        okLabel: "Permitir",
        cancelLabel: "Cancelar",
      });
      if (ok) setComputerOn(await computerEnable(true));
    } catch (e) {
      setError(String(e));
    }
  };
  onMount(() => {
    // typing lag diagnosis: stalls of the webview's main thread go to lag.log (see lag_log in the host)
    try {
      new PerformanceObserver((list) => {
        for (const e of list.getEntries()) if (e.duration >= 80) void tauriInvoke("lag_log", { what: "main thread stalled", ms: Math.round(e.duration) }).catch(() => 0);
      }).observe({ entryTypes: ["longtask"] });
    } catch {
      /* longtask not supported */
    }
  });

  onMount(() => void computerStatus().then(setComputerOn).catch(() => 0));

  // a missing git, or no agent CLI at all, is explained once at startup instead of surfacing as a cryptic failure later
  onMount(() =>
    void tauriInvoke<string[]>("missing_tools")
      .then((m) => {
        void agentsList().then((l) => setInstalledAgents(l.filter((a) => a.installed).map((a) => a.id))).catch(() => setInstalledAgents(null));
        const bits: string[] = [];
        if (m.includes("git")) bits.push("o git não está instalado ou não está no PATH (sem ele não há worktrees)");
        if (m.includes("claude") && m.includes("codex")) bits.push("nem o claude nem o codex foram encontrados (instala um deles para usar agentes)");
        if (bits.length) showNotice(`Falta configurar: ${bits.join("; ")}.`);
      })
      .catch(() => setInstalledAgents(null)),
  );

  onMount(() => {
    void refreshProjects();
    // sessions of a previous daemon run come back before the first list, so the sidebar never shows them missing
    const restore = (prefix: string) =>
      restoreSessions()
        .catch(() => null)
        .then((r) => {
          void refreshSessions();
          if (!r) return;
          const bits: string[] = [];
          if (r.restored) bits.push(`${r.restored} ${r.restored === 1 ? "sessão restaurada" : "sessões restauradas"}`);
          if (r.skipped) bits.push(`${r.skipped} ${r.skipped === 1 ? "conversa vazia ignorada" : "conversas vazias ignoradas"}`);
          if (r.failed.length) bits.push(`não foi possível restaurar: ${r.failed.join(", ")}`);
          if (bits.length) showNotice(`${prefix}: ${bits.join(" · ")}.`);
        });
    void restore("Ao reabrir");
    // the daemon crashed or was replaced while the app was open: its sessions are gone, bring them back
    let unlistenRestart: (() => void) | undefined;
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen("daemon-restarted", () => {
        showNotice("O daemon foi reiniciado; a restaurar as sessões…");
        void restore("Após o reinício do daemon");
      }).then((u) => (unlistenRestart = u)),
    );
    // the host reconnects to the daemon on its own: show it meanwhile, and say so when it did not work
    const unlistenLink: (() => void)[] = [];
    void import("@tauri-apps/api/event").then(({ listen }) => {
      void listen("daemon-reconnecting", () => setDaemonReconnecting(true)).then((u) => unlistenLink.push(u));
      void listen<{ ok: boolean }>("daemon-reconnected", (e) => {
        setDaemonReconnecting(false);
        if (!e.payload?.ok) setError("Não foi possível reconectar ao daemon. Reinicia o Axelstra se persistir.");
      }).then((u) => unlistenLink.push(u));
    });
    // the daemon pushes `session.changed` (see watchWorktree below); this poll is only the safety net
    const fast = setInterval(() => !document.hidden && void refreshSessions(), 10000);
    // git is the costly part: the change list only matters for the selected worktree, and the project
    // list (worktrees, remote) changes rarely. Both also refresh when the window regains focus
    // and after every action the user takes in the sidebar.
    // safety net only: real changes arrive as daemon events (see watchWorktree below)
    const mid = setInterval(() => !document.hidden && void refreshChanges(), 60000);
    const slow = setInterval(() => !document.hidden && void refreshProjects(), 60000);
    const onFocus = () => {
      void refreshChanges();
      void refreshProjects();
    };
    window.addEventListener("focus", onFocus);
    // clicking a desktop notification lands on the session that raised it
    let unlistenClick: (() => void) | undefined;
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen<{ worktree: string; session: number }>("notification-click", (e) => {
        void refreshSessions().then(() => openSession(e.payload.worktree, e.payload.session));
      }).then((u) => (unlistenClick = u)),
    );
    // Ctrl+B hides the project list, Ctrl+Shift+B the right panel; captured so the terminal never sees them.
    // The ones that move around the page leave dialogs, the editor and text fields alone.
    const onKey = (e: KeyboardEvent) => {
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
      // Ctrl+Tab / Ctrl+Shift+Tab cycle the tabs of this worktree, Ctrl+1..9 jump to one, Ctrl+J goes to the next agent waiting for an answer
      if (e.key === "Tab" || (/^[1-9]$/.test(e.key) && !e.shiftKey)) {
        const list = tabs();
        if (!list.length) return;
        const at = list.findIndex((t) => t.id === activeSession());
        const pick = e.key === "Tab" ? list[(at + (e.shiftKey ? -1 : 1) + list.length) % list.length] : list[Number(e.key) - 1];
        if (!pick) return;
        e.preventDefault();
        e.stopPropagation();
        showTab(pick);
        return;
      }
      if (k === "j" && !e.shiftKey) {
        e.preventDefault();
        e.stopPropagation();
        const waiting = sessions().filter((x) => x.meta?.kind === "agent" && x.meta.worktree && sessionState(x) === "waiting");
        if (!waiting.length) return showNotice("Nenhum agente está à espera de resposta.");
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
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => {
      window.removeEventListener("keydown", onKey, true);
      clearInterval(fast);
      clearInterval(mid);
      clearInterval(slow);
      window.removeEventListener("focus", onFocus);
      unlistenClick?.();
      unlistenRestart?.();
      unlistenLink.forEach((u) => u());
    });
  });

  // the daemon watches the selected worktree and tells us when files or git state changed
  createEffect(() => {
    const w = selectedWorktree();
    void watchWorktree(w, (changed) => {
      if (norm(changed) === norm(selectedWorktree())) {
        setFsTick((n) => n + 1);
        void refreshChanges();
      }
    }, () => void refreshSessions());
  });

  // dev servers started in this worktree's terminals; a process lookup, so only while something runs
  const refreshPorts = async () => {
    const w = selectedWorktree();
    if (!w || document.hidden || !sessionsOf(w).some((s) => s.running)) {
      if (ports().length) setPorts([]);
      return;
    }
    try {
      const p = (await listPorts(w)).map((x) => x.port);
      if (p.join() !== ports().join()) setPorts(p);
    } catch {
      /* transient */
    }
  };
  onMount(() => {
    const t = setInterval(() => void refreshPorts(), 8000);
    onCleanup(() => clearInterval(t));
  });
  createEffect(on(selectedWorktree, () => {
    setPorts([]);
    void refreshPorts();
  }));

  const tabs = () => sortedTabs(sessionsOf(selectedWorktree()));
  // making the session shown in the split pane the active one swaps the two, so the pane never ends up empty
  const activate = (id: number) => {
    if (id === splitSession()) setSplitSession(activeSession() !== id ? activeSession() : null);
    setActiveSession(id);
  };
  const showTab = (s: SessionInfo) =>
    guarded(() => {
      showView();
      activate(s.id);
    });

  // Terminals of the last few tabs stay mounted (hidden): switching back is instant, keeps the scroll position and
  // does not replay the history. Older ones are unmounted. Order is stable so nothing is moved in the DOM.
  const KEEP = 5;
  const [kept, setKept] = createSignal<number[]>([]);
  const lastUsed = new Map<number, number>();
  createEffect(() => {
    const id = activeSession();
    if (id === null || id === undefined) return;
    lastUsed.set(id, Date.now());
    setKept((k) => {
      const next = k.includes(id) ? k : [...k, id];
      if (next.length <= KEEP) return next;
      const oldest = next.filter((x) => x !== id).sort((a, b) => (lastUsed.get(a) ?? 0) - (lastUsed.get(b) ?? 0))[0];
      return next.filter((x) => x !== oldest);
    });
  });
  const keptLive = () => kept().filter((id) => (id !== splitSession() || id === activeSession()) && sessions().some((s) => s.id === id));
  const current = () => findWorktree(selectedWorktree());

  // reload the change list when the worktree, its project data or the compare mode changes
  createEffect(on([selectedWorktree, () => current()?.wt.path, changesMode], () => void refreshChanges()));

  // the same session in both panes would leave one of them blank (e.g. after a notification opened the split one)
  createEffect(() => {
    if (splitSession() !== null && splitSession() === activeSession()) setSplitSession(null);
  });

  // keep one terminal tab selected whenever the worktree has terminals
  createEffect(() => {
    const t = tabs();
    const a = activeSession();
    if (t.length > 0 && (a === null || !t.some((s) => s.id === a))) {
      // the main worktree is not a task: opening it never jumps into its agent, only into a terminal
      const pool = current()?.wt.is_main ? t.filter((s) => s.meta?.kind !== "agent") : t;
      if (pool.length > 0) setActiveSession(pool[0].id);
      else if (a !== null) setActiveSession(null);
    }
    if (t.length === 0 && a !== null) setActiveSession(null);
  });

  const spawnShell = async (c: NonNullable<ReturnType<typeof current>>) => {
    const [program, args] = await defaultShell();
    const { id } = await spawnSession(program, args, "shell", c.wt.path, { project: c.project.id, worktree: c.wt.path, kind: "shell" });
    await refreshSessions();
    return id;
  };

  const newTerminal = async () => {
    const c = current();
    if (!c) return;
    try {
      const id = await spawnShell(c);
      showView();
      activate(id);
    } catch (e) {
      setError(String(e));
    }
  };

  // split view: show the next terminal beside the active one, opening a fresh shell if there is only one
  const toggleSplit = async () => {
    if (splitSession() !== null) {
      setSplitSession(null);
      return;
    }
    const other = tabs().find((s) => s.id !== activeSession());
    if (other) {
      setSplitSession(other.id);
      return;
    }
    const c = current();
    if (!c) return;
    try {
      setSplitSession(await spawnShell(c));
    } catch (e) {
      setError(String(e));
    }
  };

  const newAgent = async () => {
    const c = current();
    if (!c) return;
    // asks which agent to open (or a plain terminal); a choice becomes the default
    const agent = await pickAgent();
    if (!agent) return;
    if (agent === "shell") return void newTerminal();
    try {
      const id = await launchDefaultAgent(c.project.id, c.wt.path, agent, AGENTS[agent], undefined, undefined, projectTerse(c.project.id));
      await refreshSessions();
      showView();
      activate(id);
    } catch (e) {
      setError(String(e));
    }
  };

  const close = async (s: SessionInfo) => {
    try {
      if (s.running) {
        const agent = s.meta?.kind === "agent";
        const ok = await ask(`Fechar "${tabLabel(s)}"? ${agent ? "O agente em execução é terminado." : "O terminal e o que corre nele são terminados."}`, { title: agent ? "Fechar agente" : "Fechar terminal", kind: "warning", okLabel: "Fechar", cancelLabel: "Cancelar" });
        if (!ok) return;
      }
      rememberClosed(s);
      if (s.running) await killSession(s.id);
      await removeSession(s.id);
    } catch (e) {
      setError(String(e));
    }
    await refreshSessions();
  };

  setSessionCloser(close);

  // Ctrl+Shift+T: bring back the tab closed last (a shell opens anew; an agent resumes its conversation)
  const reopenClosed = async () => {
    const t = popClosed();
    if (!t) return showNotice("Nenhum separador fechado para reabrir.");
    const target = findWorktree(t.meta?.worktree ?? null) ?? current();
    if (!target) return;
    const m = t.meta;
    try {
      const id =
        m?.kind === "agent" && m.agent
          ? await launchDefaultAgent(target.project.id, target.wt.path, t.label ?? m.agent, m.agent, { key: m.key, resume: m.resume }, undefined, projectTerse(target.project.id))
          : await spawnShell(target);
      await refreshSessions();
      openSession(target.wt.path, id);
    } catch (e) {
      setError(String(e));
    }
  };

  // tab being renamed (double click), tab being dragged and the one it hovers
  const [renaming, setRenaming] = createSignal<number | null>(null);
  const [dragTab, setDragTab] = createSignal<number | null>(null);
  const [dropOn, setDropOn] = createSignal<number | null>(null);

  // drag a panel edge; `dir` flips the delta for the right-hand panel
  const drag = (e: MouseEvent, dir: 1 | -1, start: number, set: (w: number) => void) => {
    e.preventDefault();
    const x0 = e.clientX;
    const move = (m: MouseEvent) => set(start + dir * (m.clientX - x0));
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.classList.remove("resizing");
    };
    document.body.classList.add("resizing");
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  // the dialogs for a new or removed worktree belong to the sidebar: show it first
  const showSidebarNow = () => {
    if (!showSidebar()) toggleSidebar();
  };

  // the same resize from the keyboard (arrows, Shift for bigger steps, Home/End)
  const resizeKey = (e: KeyboardEvent, dir: 1 | -1, w: number, range: { min: number; max: number }, set: (w: number) => void) => {
    const to = resizeTo(w, e.key, e.shiftKey, dir, range);
    if (to === null) return;
    e.preventDefault();
    set(to);
  };

  // the one tab Kobalte treats as selected (it keeps the roving tabindex there); each Tab also states its own `selected`
  const stripValue = () =>
    showKanban() ? "kanban" : showDocker() ? "docker" : openCommit() !== null ? "commit" : openFile() !== null ? "file" : openDiff() !== null ? "diff" : activeSession() !== null ? `tab-${activeSession()}` : "";
  // renaming ends with the new name (null = cancelled); focus goes back to the tab, not to the page
  const finishRename = (id: number, name: string | null, refocus = true) => {
    if (name !== null) renameTab(id, name);
    setRenaming(null);
    if (refocus) queueMicrotask(() => document.querySelector<HTMLElement>(`[data-tab="tab-${id}"]`)?.focus());
  };

  const baseName = (p: string) => p.split(/[\\/]/).pop() ?? p;

  return (
    <div
      class="app"
      classList={{ "with-right": current() !== null && showRightPanel(), "no-left": !showSidebar() }}
      style={{ "--left-w": `${leftW()}px`, "--right-w": `${rightW()}px` }}
    >
      <Show when={showSidebar()}>
        <div
          class="resizer"
          style={{ left: "var(--left-w)" }}
          role="separator"
          aria-orientation="vertical"
          aria-label="Largura da lista de projetos"
          aria-valuenow={leftW()}
          aria-valuemin={LEFT.min}
          aria-valuemax={LEFT.max}
          tabindex="0"
          onKeyDown={(e) => resizeKey(e, 1, leftW(), LEFT, setLeftW)}
          onMouseDown={(e) => drag(e, 1, leftW(), setLeftW)}
        />
      </Show>
      <Show when={current() && showRightPanel()}>
        <div
          class="resizer right-edge"
          style={{ right: "var(--right-w)" }}
          role="separator"
          aria-orientation="vertical"
          aria-label="Largura do painel direito"
          aria-valuenow={rightW()}
          aria-valuemin={RIGHT.min}
          aria-valuemax={RIGHT.max}
          tabindex="0"
          onKeyDown={(e) => resizeKey(e, -1, rightW(), RIGHT, setRightW)}
          onMouseDown={(e) => drag(e, -1, rightW(), setRightW)}
        />
      </Show>
      <Show when={agentPrompt()}>
        {(done) => {
          const [choice, setChoice] = createSignal(projectAgent(current()?.project.id));
          const go = () => {
            setDefaultAgent(choice());
            done()(choice());
          };
          return (
            <Modal label="Novo agente" onClose={() => done()(null)}>
              <div class="m-heading">Novo agente</div>
              <div class="m-full" onKeyDown={(e) => e.key === "Enter" && (e.ctrlKey || e.metaKey) && go()}>
                <Field label="Agente" full>
                <Select
                  value={choice()}
                  label="Agente"
                  iconOnly
                  options={[...agentKeys().map((a) => ({ value: a, label: a, icon: hasIcon(a) ? a : undefined }))]}
                  onChange={setChoice}
                />
                </Field>
              </div>
              <div class="m-actions" style={{ "justify-content": "flex-end" }}>
                <Button onClick={() => done()(null)}>Cancelar</Button>
                <Button variant="default" onClick={go} ref={(el) => queueMicrotask(() => el.focus())}>
                  Abrir
                </Button>
              </div>
            </Modal>
          );
        }}
      </Show>
      <Palette
        actions={() => [
          ...(current()
            ? [
                { label: `Novo agente (${projectAgent(current()!.project.id)})`, run: () => void newAgent() },
                { label: "Novo terminal", run: () => void newTerminal() },
                { label: "Dividir terminal", run: () => void toggleSplit() },
                { label: "Reabrir último separador fechado", hint: K("Ctrl+Shift+T"), run: () => void reopenClosed() },
                ...(activeSession() !== null ? [{ label: "Mudar o nome do separador", hint: "F2", run: () => setRenaming(activeSession()) }] : []),
                ...(current()!.wt.is_main
                  ? []
                  : [{ label: "Remover worktree atual", hint: "worktree", run: () => { showSidebarNow(); setRemoveRequest({ project: current()!.project.id, path: current()!.wt.path, name: current()!.wt.branch ?? current()!.wt.path }); } }]),
              ]
            : []),
          ...(projects().length > 0
            ? [{ label: "Novo worktree", hint: "worktree", run: () => { showSidebarNow(); setCreatingWorktree(current()?.project.id ?? projects()[0].id); } }]
            : []),
          { label: "Pesquisar conversas", hint: K("Ctrl+Shift+F"), run: () => setShowAgentSearch(true) },
          { label: "Orquestração: agentes e workers", run: () => setShowOrchestra(true) },
          { label: "Definições", hint: K("Ctrl+,"), run: () => setShowSettings(true) },
          { label: "Despachante", hint: "pedidos enviados a cada worktree", run: () => setShowDispatcher(true) },
          { label: "Automatizações", hint: "agendamento", run: () => setShowAutomations(true) },
          { label: "Atalhos de teclado", hint: "?", run: () => setShowShortcuts(true) },
          { label: showSidebar() ? "Ocultar projetos" : "Mostrar projetos", hint: K("Ctrl+B"), run: toggleSidebar },
          { label: showRightPanel() ? "Ocultar painel direito" : "Mostrar painel direito", hint: K("Ctrl+Shift+B"), run: toggleRightPanel },
          { label: computerOn() ? "Controlo do ecrã pelos agentes: desligar" : "Controlo do ecrã pelos agentes: ligar", run: () => void toggleComputer() },
        ]}
      />
      <Show when={showShortcuts()}>
        <Shortcuts />
      </Show>
      <Show when={showAgentSearch()}>
        <AgentSearch />
      </Show>
      <Show when={showOrchestra()}>
        <Orchestra />
      </Show>
      <Show when={showSettings()}>
        <Settings />
      </Show>
      <Show when={showProjectSettings()} keyed>
        {(id) => <ProjectSettings projectId={id} onClose={() => setShowProjectSettings(null)} />}
      </Show>
      <Show when={showDispatcher()}>
        <Dispatcher />
      </Show>
      <Show when={showAutomations()}>
        <Automations />
      </Show>
      <ContextMenu />
      <ErrorToast />
      <UpdateToast />
      <CloseConfirm />
      <ConfirmHost />
      <Show when={showSidebar()}>
        <Sidebar />
      </Show>
      <main class="main">
        <Show
          when={current()}
          fallback={
            <div class="placeholder">
              <Show when={!showSidebar()}>
                <Button variant="icon" size="tool" title={K("Mostrar projetos (Ctrl+B)")} aria-label={K("Mostrar projetos (Ctrl+B)")} onClick={toggleSidebar}>
                  <PanelIcon side="left" on={false} />
                </Button>
              </Show>
              <div>Adiciona um projeto e escolhe um worktree.</div>
              <Show when={projects().length === 0}>
                <Button variant="default" onClick={() => void pickProject()}>Adicionar projeto</Button>
              </Show>
            </div>
          }
        >
          <div class="tabbar">
            <Button variant="icon" size="tool" class={cn("ml-1 self-center", showSidebar() && "text-foreground")} title={K("Projetos (Ctrl+B)")} aria-label={K("Projetos (Ctrl+B)")} onClick={toggleSidebar}>
              <PanelIcon side="left" on={showSidebar()} />
            </Button>
            <Tabs value={stripValue()} onChange={() => {}} activationMode="manual" class="flex min-w-0">
            <TabsList aria-label="Separadores" class="min-w-0 overflow-x-auto [scrollbar-width:none]">
            <Tab value="kanban" selected={showKanban()} onSelect={() => guarded(() => { setShowDocker(false); setShowKanban(true); })}>
              <span class="dtag"><Icon name="grid" size={11} /></span>
              <span>Kanban</span>
            </Tab>
            <Tab value="docker" selected={showDocker()} onSelect={() => guarded(() => { setShowKanban(false); setShowDocker(true); })}>
              <span class="dtag"><Icon name="box" size={11} /></span>
              <span>Docker</span>
            </Tab>
            <For each={tabs()}>
              {(s) => (
                <Tab
                  id={`tab-${s.id}`}
                  value={`tab-${s.id}`}
                  selected={!showKanban() && !showDocker() && openDiff() === null && openFile() === null && openCommit() === null && activeSession() === s.id}
                  onSelect={() => showTab(s)}
                  onClose={() => void close(s)}
                  onRename={() => setRenaming(s.id)}
                  dropBefore={dropOn() === s.id && dragTab() !== s.id}
                  wrap={{
                    draggable: renaming() !== s.id,
                    onDragStart: (e) => {
                      setDragTab(s.id);
                      e.dataTransfer?.setData("text/plain", String(s.id));
                    },
                    onDragOver: (e) => {
                      if (dragTab() === null) return;
                      e.preventDefault();
                      setDropOn(s.id);
                    },
                    onDrop: (e) => {
                      e.preventDefault();
                      const from = dragTab();
                      if (from !== null) moveTab(tabs().map((t) => t.id), from, s.id);
                      setDragTab(null);
                      setDropOn(null);
                    },
                    onDragEnd: () => {
                      setDragTab(null);
                      setDropOn(null);
                    },
                    onDblClick: () => setRenaming(s.id),
                    onMouseDown: (e) => e.button === 1 && e.preventDefault(),
                    onAuxClick: (e) => {
                      if (e.button !== 1) return;
                      e.preventDefault();
                      void close(s);
                    },
                  }}
                  editing={
                    renaming() === s.id ? (
                      <>
                        <StateDot state={dotOfSession(s)} />
                        <Input
                          class="h-auto w-[120px] rounded-sm border-border bg-transparent px-1 py-0 text-inherit"
                          aria-label="Nome do separador"
                          value={tabLabel(s)}
                          onClick={(e) => e.stopPropagation()}
                          onKeyDown={(e) => {
                            e.stopPropagation();
                            if (e.key === "Enter") finishRename(s.id, e.currentTarget.value);
                            else if (e.key === "Escape") finishRename(s.id, null);
                          }}
                          onBlur={(e) => renaming() === s.id && finishRename(s.id, e.currentTarget.value, false)}
                          ref={(el) => queueMicrotask(() => { el.focus(); el.select(); })}
                        />
                      </>
                    ) : undefined
                  }
                >
                  <StateDot state={dotOfSession(s)} />
                  <span>{s.meta?.worker ? <Icon name="subArrow" size={11} inline /> : null}{tabLabel(s)}</span>
                </Tab>
              )}
            </For>
            <Show when={openCommit()}>
              {(c) => (
                <Tab value="commit" class="diff-tab" selected={!showKanban() && !showDocker()} title={c().subject} onSelect={() => { setShowKanban(false); setShowDocker(false); }} onClose={() => setOpenCommit(null)} closeLabel="Fechar commit">
                  <span class="dtag"><Icon name="branch" size={11} /></span>
                  <span>{c().short}</span>
                </Tab>
              )}
            </Show>
            <Show when={openFile()}>
              {(p) => (
                <Tab value="file" class="diff-tab" selected={!showKanban() && !showDocker()} title={p()} onSelect={() => { setShowKanban(false); setShowDocker(false); }} onClose={() => guarded(() => setOpenFile(null))} closeLabel="Fechar ficheiro">
                  <span class="dtag"><Icon name="pencil" size={11} /></span>
                  <span>{baseName(p())}</span>
                </Tab>
              )}
            </Show>
            <Show when={openDiff()}>
              {(p) => (
                <Tab value="diff" class="diff-tab" selected={!showKanban() && !showDocker()} title={p()} onSelect={() => { setShowKanban(false); setShowDocker(false); }} onClose={() => setOpenDiff(null)} closeLabel="Fechar diff">
                  <span class="dtag">Δ</span>
                  <span>{baseName(p())}</span>
                </Tab>
              )}
            </Show>
            </TabsList>
            </Tabs>
            <span class="grow" />
            <UsageChip />
            <Show when={computerOn()}>
              <span class="computer-on" title="Os agentes podem ver o ecrã e controlar o rato e o teclado. Desliga em: paleta de comandos, «Controlo do ecrã»"><i class="bdot" /> ecrã</span>
            </Show>
            <For each={ports()}>
              {(port) => (
                <Button variant="success" size="chip" class="ml-1.5 self-center rounded-xl" title={`Abrir http://localhost:${port} no navegador`} onClick={() => void openUrl(`http://localhost:${port}`)}>
                  :{port} <Icon name="external" size={11} inline />
                </Button>
              )}
            </For>
            <Button variant="icon" size="tool" class={cn("ml-1 self-center", listening() && "text-destructive")} title={K("Ditado por voz (Ctrl+Shift+M)")} aria-label={K("Ditado por voz (Ctrl+Shift+M)")} onClick={() => void toggleVoice()} hidden={!voiceSupported}>
              <MicIcon />
            </Button>
            <Button variant="icon" size="tool" class={cn("ml-1 self-center", splitSession() !== null && "text-foreground")} title="Dividir: ver dois terminais lado a lado" aria-label="Dividir: ver dois terminais lado a lado" onClick={() => void toggleSplit()}>
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
                <path d="M5.5 3v10M10.5 3v10" />
              </svg>
            </Button>
            <Button variant="icon" size="tool" class="ml-1 self-center" title={`Novo agente (${projectAgent(current()?.project.id)})`} aria-label={`Novo agente (${projectAgent(current()?.project.id)})`} onClick={newAgent}>
              <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
                <path d="M8 1.5c.4 3.4 1.6 4.6 5 5-3.4.4-4.6 1.6-5 5-.4-3.4-1.6-4.6-5-5 3.4-.4 4.6-1.6 5-5z" />
              </svg>
            </Button>
            <Button variant="icon" size="tool" class="ml-1 self-center" title="Novo terminal" aria-label="Novo terminal" onClick={newTerminal}>
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
                <path d="M8 3.5v9M3.5 8h9" />
              </svg>
            </Button>
            <Show when={!showRightPanel()}>
              <Button variant="icon" size="tool" class="ml-1 self-center" title={K("Mostrar painel direito (Ctrl+Shift+B)")} aria-label={K("Mostrar painel direito (Ctrl+Shift+B)")} onClick={toggleRightPanel}>
                <PanelIcon side="right" on={false} />
              </Button>
            </Show>
          </div>
          <VoiceDraft target={activeSession()} />
          <div class="term-area">
            <Show when={!showDocker()} fallback={<DockerView />}>
            <Show when={!showKanban()} fallback={<Kanban project={current()!.project.id} />}>
            <Show when={openCommit() === null} fallback={<CommitView />}>
            <Show
              when={openFile() === null}
              fallback={
                <Show when={openFile()} keyed>
                  {(f) => <Editor wt={selectedWorktree()!} path={f} initialLine={fileLine() ?? undefined} />}
                </Show>
              }
            >
            <Show
              when={openDiff() === null}
              fallback={<DiffView />}
            >
              <Show
                when={activeSession() !== null}
                fallback={
                  <div class="placeholder empty">
                    <div>Sem terminais neste worktree.</div>
                    <Show when={current()}>{(c) => <WorktreeReports project={c().project.id} worktree={c().wt.path} />}</Show>
                    <div class="empty-actions">
                      <Button variant="default" onClick={() => void newAgent()}>
                        <Icon name="sparkle" size={12} inline /> Abrir agente
                      </Button>
                      <Button onClick={() => void newTerminal()}>+ Terminal</Button>
                    </div>
                    <div class="hints">
                      <kbd>{K("Ctrl+K")}</kbd> paleta · <kbd>{K("Ctrl+B")}</kbd> projetos · <kbd>{K("Ctrl+Shift+B")}</kbd> painel direito
                    </div>
                  </div>
                }
              >
                <For each={keptLive()}>
                  {(id) => (
                    <div class="term-slot" style={{ display: id === activeSession() ? "flex" : "none" }}>
                      <TerminalView id={id} active={() => id === activeSession()} onExit={() => void refreshSessions()} />
                    </div>
                  )}
                </For>
                <Show when={splitSession() !== null && splitSession() !== activeSession() && tabs().some((s) => s.id === splitSession())}>
                  <div
                    class="split-sep"
                    role="separator"
                    aria-orientation="vertical"
                    aria-label="Largura do terminal dividido"
                    aria-valuenow={Math.round(splitFrac() * 100)}
                    aria-valuemin={20}
                    aria-valuemax={80}
                    tabindex="0"
                    onKeyDown={(e) => {
                      const to = splitTo(splitFrac(), e.key);
                      if (to === null) return;
                      e.preventDefault();
                      setSplitFrac(to);
                    }}
                    onMouseDown={(e) => {
                      e.preventDefault();
                      const area = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
                      const move = (m: MouseEvent) => setSplitFrac(Math.min(0.8, Math.max(0.2, (area.right - m.clientX) / area.width)));
                      const up = () => {
                        window.removeEventListener("mousemove", move);
                        window.removeEventListener("mouseup", up);
                        document.body.classList.remove("resizing");
                      };
                      document.body.classList.add("resizing");
                      window.addEventListener("mousemove", move);
                      window.addEventListener("mouseup", up);
                    }}
                  />
                  <div class="split-pane group/split" style={{ flex: `0 0 ${splitFrac() * 100}%` }}>
                    <Button variant="outline" size="bare" class="absolute right-2 top-1 z-[5] size-5 border-border/60 bg-popover leading-none text-muted-foreground opacity-0 transition-opacity focus-visible:opacity-100 group-hover/split:opacity-100" title="Fechar divisão" aria-label="Fechar divisão" onClick={() => setSplitSession(null)}>
                      <Icon name="x" />
                    </Button>
                    <Show when={splitSession()} keyed>
                      {(id) => <TerminalView id={id} focus={false} onExit={() => void refreshSessions()} />}
                    </Show>
                  </div>
                </Show>
              </Show>
            </Show>
            </Show>
            </Show>
            </Show>
            </Show>
          </div>
        </Show>
      </main>
      <Show when={current() && showRightPanel()}>
        <Show when={rightTab() === "files"} fallback={rightTab() === "pulls" ? <PullsPanel /> : <ChangesPanel />}>
          <FilesPanel />
        </Show>
      </Show>
      <StatusBar />
    </div>
  );
}
