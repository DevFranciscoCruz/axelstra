import { For, Show, createSignal, onCleanup } from "solid-js";
import { AgentHit, agentSearch, launchDefaultAgent } from "./api";
import { Modal } from "./Modal";
import { findWorktree, refreshSessions, setError, setShowAgentSearch, setActiveSession, selectWorktree } from "./store";

const baseName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;
const ago = (ts: number) => {
  const d = Math.max(0, Date.now() / 1000 - ts);
  return d < 3600 ? `${Math.max(1, Math.round(d / 60))} min` : d < 86400 ? `${Math.round(d / 3600)} h` : `${Math.round(d / 86400)} d`;
};

/** full-text search over past Claude/Codex conversations; a hit in a known worktree reopens that conversation */
export function AgentSearch() {
  const [q, setQ] = createSignal("");
  const [hits, setHits] = createSignal<AgentHit[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [done, setDone] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  onCleanup(() => clearTimeout(timer));

  const run = (text: string) => {
    clearTimeout(timer);
    setDone(false);
    if (text.trim().length < 2) return setHits([]);
    timer = setTimeout(async () => {
      const mine = ++seq;
      setBusy(true);
      try {
        const r = await agentSearch(text.trim());
        if (mine === seq) {
          setHits(r);
          setDone(true);
        }
      } catch (e) {
        if (mine === seq) setError(String(e).includes("unknown method") ? "Reinicie o daemon para usar a pesquisa de conversas." : String(e));
      } finally {
        if (mine === seq) setBusy(false);
      }
    }, 300);
  };

  const target = (h: AgentHit) => (h.agent === "claude" ? findWorktree(h.cwd) : null);
  const open = async (h: AgentHit) => {
    const t = target(h);
    if (!t) return;
    setShowAgentSearch(false);
    selectWorktree(t.wt.path);
    try {
      const id = await launchDefaultAgent(t.project.id, t.wt.path, "claude", "claude", { resume: h.session });
      await refreshSessions();
      setActiveSession(id);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <Modal label="Pesquisar conversas" onClose={() => setShowAgentSearch(false)} class="modal auto">
      <input
        class="m-title"
        placeholder="Pesquisar nas conversas do Claude e do Codex…"
        value={q()}
        onInput={(e) => {
          setQ(e.currentTarget.value);
          run(e.currentTarget.value);
        }}
        ref={(el) => queueMicrotask(() => el.focus())}
      />
      <div class="auto-list">
        <For each={hits()}>
          {(h) => (
            <div class="auto-item search-hit" classList={{ off: !target(h) }} onClick={() => void open(h)} title={target(h) ? "Retomar esta conversa" : h.cwd}>
              <div class="auto-main">
                <div class="auto-name">
                  {h.agent} · {baseName(h.cwd || "?")} <span class="auto-note">há {ago(h.ts)}</span>
                </div>
                <div class="auto-note">{h.snippet}</div>
              </div>
            </div>
          )}
        </For>
        <Show when={done() && hits().length === 0}>
          <div class="auto-note">Nada encontrado.</div>
        </Show>
        <Show when={busy()}>
          <div class="auto-note">A pesquisar…</div>
        </Show>
      </div>
      <div class="auto-note">Só as conversas do Claude em worktrees conhecidos podem ser retomadas aqui; as restantes aparecem a cinzento.</div>
    </Modal>
  );
}
