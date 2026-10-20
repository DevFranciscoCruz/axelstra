import { For, Show, createEffect, createSignal, on, onCleanup, onMount } from "solid-js";
import { SessionInfo, restoreSessions, listPorts, watchWorktree, defaultShell, launchDefaultAgent, killSession, removeSession, spawnSession } from "./api";
import { ChangesPanel } from "./ChangesPanel";
import { CommitView } from "./CommitView";
import { Editor } from "./Editor";
import { FilesPanel } from "./FilesPanel";
import { Kanban } from "./Kanban";
import { DiffView } from "./DiffView";
import { Sidebar } from "./Sidebar";
import { Palette } from "./Palette";
import { ContextMenu } from "./ContextMenu";
import { Automations } from "./Automations";
import { openUrl } from "@tauri-apps/plugin-opener";
import { UpdateToast } from "./UpdateToast";
import { PanelIcon } from "./PanelIcon";
import { ErrorToast } from "./ErrorToast";
import { Modal } from "./Modal";
import { labelIcons, press } from "./a11y";
import { ask } from "@tauri-apps/plugin-dialog";
import { PullsPanel } from "./PullsPanel";
import { TerminalView } from "./TerminalView";
import {
  activeSession,
  AGENTS,
  agentPrompt,
  defaultAgent,
  setDefaultAgent,
  changesMode,
  showAutomations,
  ports,
  setPorts,
  splitSession,
  setSplitSession,
  setFsTick,
  norm,
  findWorktree,
  openDiff,
  openCommit,
  openFile,
  rightTab,
  setOpenCommit,
  setOpenFile,
  setShowKanban,
  showKanban,
  refreshChanges,
  refreshProjects,
  refreshSessions,
  selectedWorktree,
  sessionsOf,
  setActiveSession,
  setError,
  setOpenDiff,
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
  onMount(() => {
    labelIcons();
    void refreshProjects();
    // sessions of a previous daemon run come back before the first list, so the sidebar never shows them missing
    void restoreSessions().catch(() => 0).then(() => refreshSessions());
    // The daemon has no push events yet: poll only while the window is visible.
    const fast = setInterval(() => !document.hidden && void refreshSessions(), 2000);
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
    // Ctrl+B hides the project list, Ctrl+Shift+B the right panel; captured so the terminal never sees them
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.key.toLowerCase() !== "b") return;
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
    });
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

  const tabs = () => sessionsOf(selectedWorktree());
  const current = () => findWorktree(selectedWorktree());

  // reload the change list when the worktree, its project data or the compare mode changes
  createEffect(on([selectedWorktree, () => current()?.wt.path, changesMode], () => void refreshChanges()));

  // keep one terminal tab selected whenever the worktree has terminals
  createEffect(() => {
    const t = tabs();
    const a = activeSession();
    if (t.length > 0 && (a === null || !t.some((s) => s.id === a))) setActiveSession(t[0].id);
    if (t.length === 0 && a !== null) setActiveSession(null);
  });

  const newTerminal = async () => {
    const c = current();
    if (!c) return;
    try {
      const [program, args] = await defaultShell();
      const { id } = await spawnSession(program, args, "shell", c.wt.path, {
        project: c.project.id,
        worktree: c.wt.path,
        kind: "shell",
      });
      await refreshSessions();
      setOpenDiff(null);
      setActiveSession(id);
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
      const [program, args] = await defaultShell();
      const { id } = await spawnSession(program, args, "shell", c.wt.path, { project: c.project.id, worktree: c.wt.path, kind: "shell" });
      await refreshSessions();
      setSplitSession(id);
    } catch (e) {
      setError(String(e));
    }
  };

  const newAgent = async () => {
    const c = current();
    if (!c) return;
    try {
      const id = await launchDefaultAgent(c.project.id, c.wt.path, defaultAgent(), AGENTS[defaultAgent()]);
      await refreshSessions();
      setOpenDiff(null);
      setActiveSession(id);
    } catch (e) {
      setError(String(e));
    }
  };

  const close = async (s: SessionInfo) => {
    if (s.running && s.meta?.kind === "agent") {
      const ok = await ask(`Fechar "${s.label ?? `Sessão ${s.id}`}"? O agente em execução é terminado.`, { title: "Fechar agente", kind: "warning", okLabel: "Fechar", cancelLabel: "Cancelar" });
      if (!ok) return;
    }
    if (s.running) await killSession(s.id);
    await removeSession(s.id);
    await refreshSessions();
  };

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

  const baseName = (p: string) => p.split(/[\\/]/).pop() ?? p;

  return (
    <div
      class="app"
      classList={{ "with-right": current() !== null && showRightPanel(), "no-left": !showSidebar() }}
      style={{ "--left-w": `${leftW()}px`, "--right-w": `${rightW()}px` }}
    >
      <Show when={showSidebar()}>
        <div class="resizer" style={{ left: "var(--left-w)" }} onMouseDown={(e) => drag(e, 1, leftW(), setLeftW)} />
      </Show>
      <Show when={current() && showRightPanel()}>
        <div class="resizer right-edge" style={{ right: "var(--right-w)" }} onMouseDown={(e) => drag(e, -1, rightW(), setRightW)} />
      </Show>
      <Show when={agentPrompt()}>
        {(done) => (
          <Modal label="Abrir com qual agente?" onClose={() => done()(null)} backClass="agent-back" class="agent-modal">
            <h3>Abrir com qual agente?</h3>
            <For each={Object.keys(AGENTS)}>
              {(a) => (
                <button
                  class="primary"
                  classList={{ pref: a === defaultAgent() }}
                  onClick={() => {
                    setDefaultAgent(a);
                    done()(a);
                  }}
                  ref={(el) => a === defaultAgent() && queueMicrotask(() => el.focus())}
                >
                  {a}
                </button>
              )}
            </For>
            <button onClick={() => done()("shell")}>Só terminal</button>
          </Modal>
        )}
      </Show>
      <Palette
        actions={() =>
          current()
            ? [
                { label: `Novo agente (${defaultAgent()})`, run: () => void newAgent() },
                { label: "Novo terminal", run: () => void newTerminal() },
                { label: "Dividir terminal", run: () => void toggleSplit() },
              ]
            : []
        }
      />
      <Show when={showAutomations()}>
        <Automations />
      </Show>
      <ContextMenu />
      <ErrorToast />
      <UpdateToast />
      <Show when={showSidebar()}>
        <Sidebar />
      </Show>
      <main class="main">
        <Show
          when={current()}
          fallback={
            <div class="placeholder">
              <Show when={!showSidebar()}>
                <button class="icon add" title="Mostrar projetos (Ctrl+B)" onClick={toggleSidebar}>
                  <PanelIcon side="left" on={false} />
                </button>
              </Show>
              Adicione um projeto e escolha um worktree.
            </div>
          }
        >
          <div class="tabbar">
            <button class="icon add" title="Projetos (Ctrl+B)" classList={{ on: showSidebar() }} onClick={toggleSidebar}>
              <PanelIcon side="left" on={showSidebar()} />
            </button>
            <div class="tabs">
            <div class="tab" classList={{ active: showKanban() }} {...press(() => guarded(() => setShowKanban(true)), "tab")}>
              <span class="dtag">▦</span>
              <span>Kanban</span>
            </div>
            <For each={tabs()}>
              {(s) => (
                <div
                  class="tab"
                  classList={{ active: !showKanban() && openDiff() === null && openFile() === null && openCommit() === null && activeSession() === s.id }}
                  {...press(
                    () =>
                      guarded(() => {
                        setOpenDiff(null);
                        setOpenFile(null);
                        setOpenCommit(null);
                        setShowKanban(false);
                        setActiveSession(s.id);
                      }),
                    "tab",
                  )}
                >
                  <span class="dot" classList={{ on: s.running }} />
                  <span>{s.label ?? `Sessão ${s.id}`}</span>
                  <button
                    class="x"
                    title="Fechar"
                    onClick={(e) => {
                      e.stopPropagation();
                      void close(s);
                    }}
                  >
                    ×
                  </button>
                </div>
              )}
            </For>
            <Show when={openCommit()}>
              {(c) => (
                <div class="tab diff-tab" classList={{ active: !showKanban() }} title={c().subject} {...press(() => setShowKanban(false), "tab")}>
                  <span class="dtag">⎇</span>
                  <span>{c().short}</span>
                  <button class="x" title="Fechar commit" onClick={() => setOpenCommit(null)}>
                    ×
                  </button>
                </div>
              )}
            </Show>
            <Show when={openFile()}>
              {(p) => (
                <div class="tab diff-tab" classList={{ active: !showKanban() }} title={p()} {...press(() => setShowKanban(false), "tab")}>
                  <span class="dtag">✎</span>
                  <span>{baseName(p())}</span>
                  <button class="x" title="Fechar ficheiro" onClick={(e) => { e.stopPropagation(); guarded(() => setOpenFile(null)); }}>
                    ×
                  </button>
                </div>
              )}
            </Show>
            <Show when={openDiff()}>
              {(p) => (
                <div class="tab diff-tab" classList={{ active: !showKanban() }} title={p()} {...press(() => setShowKanban(false), "tab")}>
                  <span class="dtag">Δ</span>
                  <span>{baseName(p())}</span>
                  <button class="x" title="Fechar diff" onClick={() => setOpenDiff(null)}>
                    ×
                  </button>
                </div>
              )}
            </Show>
            </div>
            <span class="grow" />
            <For each={ports()}>
              {(port) => (
                <button class="port" title={`Abrir http://localhost:${port} no navegador`} onClick={() => void openUrl(`http://localhost:${port}`)}>
                  :{port} ↗
                </button>
              )}
            </For>
            <button class="icon add" title="Dividir: ver dois terminais lado a lado" classList={{ on: splitSession() !== null }} onClick={() => void toggleSplit()}>
              ⫼
            </button>
            <button class="icon add" title={`Novo agente (${defaultAgent()})`} onClick={newAgent}>
              ✦
            </button>
            <button class="icon add" title="Novo terminal" onClick={newTerminal}>
              +
            </button>
            <Show when={!showRightPanel()}>
              <button class="icon add" title="Mostrar painel direito (Ctrl+Shift+B)" onClick={toggleRightPanel}>
                <PanelIcon side="right" on={false} />
              </button>
            </Show>
          </div>
          <div class="term-area">
            <Show when={!showKanban()} fallback={<Kanban project={current()!.project.id} />}>
            <Show when={openCommit() === null} fallback={<CommitView />}>
            <Show
              when={openFile() === null}
              fallback={
                <Show when={openFile()} keyed>
                  {(f) => <Editor wt={selectedWorktree()!} path={f} />}
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
                    <div class="empty-actions">
                      <button class="primary" onClick={() => void newAgent()}>
                        ✦ Abrir agente
                      </button>
                      <button onClick={() => void newTerminal()}>+ Terminal</button>
                    </div>
                    <div class="hints">
                      <kbd>Ctrl+K</kbd> paleta · <kbd>Ctrl+B</kbd> projetos · <kbd>Ctrl+Shift+B</kbd> painel direito
                    </div>
                  </div>
                }
              >
                {/* keyed: switching tab remounts the terminal and replays its history */}
                <Show when={activeSession()} keyed>
                  {(id) => <TerminalView id={id} onExit={() => void refreshSessions()} />}
                </Show>
                <Show when={splitSession() !== null && splitSession() !== activeSession() && tabs().some((s) => s.id === splitSession())}>
                  <div
                    class="split-sep"
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
                  <div class="split-pane" style={{ flex: `0 0 ${splitFrac() * 100}%` }}>
                    <button class="split-x" title="Fechar divisão" onClick={() => setSplitSession(null)}>
                      ×
                    </button>
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
          </div>
        </Show>
      </main>
      <Show when={current() && showRightPanel()}>
        <Show when={rightTab() === "files"} fallback={rightTab() === "pulls" ? <PullsPanel /> : <ChangesPanel />}>
          <FilesPanel />
        </Show>
      </Show>
    </div>
  );
}
