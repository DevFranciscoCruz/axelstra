import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { SessionInfo, killSession, listSessions, removeSession, spawnSession } from "./api";
import { TerminalView } from "./TerminalView";
import logo from "./assets/logo.png";

export function App() {
  const [sessions, setSessions] = createSignal<SessionInfo[]>([]);
  const [active, setActive] = createSignal<number | null>(null);
  const [error, setError] = createSignal("");

  const refresh = async () => {
    try {
      setSessions(await listSessions());
      setError("");
    } catch (e) {
      setError(String(e));
    }
  };

  onMount(() => {
    void refresh();
    // The daemon has no push event for new sessions yet; refresh only while the window is visible.
    const t = setInterval(() => {
      if (!document.hidden) void refresh();
    }, 2000);
    onCleanup(() => clearInterval(t));
  });

  const newTerminal = async () => {
    try {
      const { id } = await spawnSession("powershell", ["-NoLogo"], "shell");
      await refresh();
      setActive(id);
    } catch (e) {
      setError(String(e));
    }
  };

  const close = async (s: SessionInfo) => {
    if (s.running) await killSession(s.id);
    await removeSession(s.id);
    if (active() === s.id) setActive(null);
    await refresh();
  };

  return (
    <div class="app">
      <aside class="sidebar">
        <div class="brand">
          <img src={logo} alt="" />
          Axelstra
        </div>
        <button class="primary" onClick={newTerminal}>
          + Novo terminal
        </button>
        <div class="section">Sessões</div>
        <For each={sessions()} fallback={<div class="empty">Sem sessões</div>}>
          {(s) => (
            <div class="item" classList={{ active: active() === s.id }} onClick={() => setActive(s.id)}>
              <span class="dot" classList={{ on: s.running }} />
              <span class="name">{s.label ?? `sessão ${s.id}`}</span>
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
        <Show when={error()}>
          <div class="error">{error()}</div>
        </Show>
      </aside>
      <main class="main">
        <Show when={active() !== null} fallback={<div class="placeholder">Escolha ou crie um terminal</div>}>
          {/* keyed: switching session remounts the terminal and replays its history */}
          <Show when={active()} keyed>
            {(id) => <TerminalView id={id} onExit={() => void refresh()} />}
          </Show>
        </Show>
      </main>
    </div>
  );
}
