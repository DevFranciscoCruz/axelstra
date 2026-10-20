import { For, Show, createEffect, onCleanup, onMount } from "solid-js";
import { SessionInfo, killSession, removeSession, spawnSession } from "./api";
import { Sidebar } from "./Sidebar";
import { TerminalView } from "./TerminalView";
import {
  activeSession,
  findWorktree,
  refreshProjects,
  refreshSessions,
  selectedWorktree,
  sessionsOf,
  setActiveSession,
  setError,
} from "./store";

export function App() {
  onMount(() => {
    void refreshProjects();
    void refreshSessions();
    // The daemon has no push events yet: poll only while the window is visible.
    const fast = setInterval(() => !document.hidden && void refreshSessions(), 2000);
    const slow = setInterval(() => !document.hidden && void refreshProjects(), 15000);
    onCleanup(() => {
      clearInterval(fast);
      clearInterval(slow);
    });
  });

  const tabs = () => sessionsOf(selectedWorktree());
  const current = () => findWorktree(selectedWorktree());

  // keep one tab selected whenever the worktree has terminals
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

  return (
    <div class="app">
      <Sidebar />
      <main class="main">
        <Show
          when={current()}
          fallback={<div class="placeholder">Adicione um projeto e escolha um worktree</div>}
        >
          <div class="tabbar">
            <For each={tabs()}>
              {(s) => (
                <div class="tab" classList={{ active: activeSession() === s.id }} onClick={() => setActiveSession(s.id)}>
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
            <button class="icon add" title="Novo terminal" onClick={newTerminal}>
              +
            </button>
          </div>
          <div class="term-area">
            <Show
              when={activeSession() !== null}
              fallback={<div class="placeholder">Sem terminais neste worktree. Clique em + para abrir um.</div>}
            >
              {/* keyed: switching tab remounts the terminal and replays its history */}
              <Show when={activeSession()} keyed>
                {(id) => <TerminalView id={id} onExit={() => void refreshSessions()} />}
              </Show>
            </Show>
          </div>
        </Show>
      </main>
    </div>
  );
}
