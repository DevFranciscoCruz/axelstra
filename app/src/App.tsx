import { For, Show, createEffect, on, onCleanup, onMount } from "solid-js";
import { SessionInfo, killSession, removeSession, spawnSession } from "./api";
import { ChangesPanel } from "./ChangesPanel";
import { Editor } from "./Editor";
import { FilesPanel } from "./FilesPanel";
import { Kanban } from "./Kanban";
import { DiffView } from "./DiffView";
import { Sidebar } from "./Sidebar";
import { TerminalView } from "./TerminalView";
import {
  activeSession,
  changesMode,
  findWorktree,
  openDiff,
  openFile,
  rightTab,
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
} from "./store";

export function App() {
  onMount(() => {
    void refreshProjects();
    void refreshSessions();
    // The daemon has no push events yet: poll only while the window is visible.
    const fast = setInterval(() => !document.hidden && void refreshSessions(), 2000);
    const mid = setInterval(() => !document.hidden && void refreshChanges(), 5000);
    const slow = setInterval(() => !document.hidden && void refreshProjects(), 15000);
    onCleanup(() => {
      clearInterval(fast);
      clearInterval(mid);
      clearInterval(slow);
    });
  });

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
      const { id } = await spawnSession("powershell", ["-NoLogo"], "shell", c.wt.path, {
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

  const close = async (s: SessionInfo) => {
    if (s.running) await killSession(s.id);
    await removeSession(s.id);
    await refreshSessions();
  };

  const baseName = (p: string) => p.split(/[\\/]/).pop() ?? p;

  return (
    <div class="app" classList={{ "with-right": current() !== null }}>
      <Sidebar />
      <main class="main">
        <Show when={current()} fallback={<div class="placeholder">Adicione um projeto e escolha um worktree</div>}>
          <div class="tabbar">
            <div class="tab" classList={{ active: showKanban() }} onClick={() => setShowKanban(true)}>
              <span class="dtag">▦</span>
              <span>Kanban</span>
            </div>
            <For each={tabs()}>
              {(s) => (
                <div
                  class="tab"
                  classList={{ active: !showKanban() && openDiff() === null && openFile() === null && activeSession() === s.id }}
                  onClick={() => {
                    setOpenDiff(null);
                    setOpenFile(null);
                    setShowKanban(false);
                    setActiveSession(s.id);
                  }}
                >
                  <span class="dot" classList={{ on: s.running }} />
                  <span>{s.label ?? `sessão ${s.id}`}</span>
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
            <Show when={openFile()}>
              {(p) => (
                <div class="tab active diff-tab" title={p()}>
                  <span class="dtag">✎</span>
                  <span>{baseName(p())}</span>
                  <button class="x" title="Fechar ficheiro" onClick={() => setOpenFile(null)}>
                    ×
                  </button>
                </div>
              )}
            </Show>
            <Show when={openDiff()}>
              {(p) => (
                <div class="tab active diff-tab" title={p()}>
                  <span class="dtag">Δ</span>
                  <span>{baseName(p())}</span>
                  <button class="x" title="Fechar diff" onClick={() => setOpenDiff(null)}>
                    ×
                  </button>
                </div>
              )}
            </Show>
            <button class="icon add" title="Novo terminal" onClick={newTerminal}>
              +
            </button>
          </div>
          <div class="term-area">
            <Show when={!showKanban()} fallback={<Kanban project={current()!.project.id} />}>
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
                fallback={<div class="placeholder">Sem terminais neste worktree. Clique em + para abrir um.</div>}
              >
                {/* keyed: switching tab remounts the terminal and replays its history */}
                <Show when={activeSession()} keyed>
                  {(id) => <TerminalView id={id} onExit={() => void refreshSessions()} />}
                </Show>
              </Show>
            </Show>
            </Show>
            </Show>
          </div>
        </Show>
      </main>
      <Show when={current()}>
        <Show when={rightTab() === "files"} fallback={<ChangesPanel />}>
          <FilesPanel />
        </Show>
      </Show>
    </div>
  );
}
