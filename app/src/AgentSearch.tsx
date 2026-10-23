import { For, Show, createEffect, createSignal, onCleanup } from "solid-js";
import { AgentHit, agentSearch, launchDefaultAgent } from "./api";
import { moveIndex } from "./a11y";
import { ago } from "./time";
import { Modal } from "./Modal";
import { findWorktree, projectTerse, refreshSessions, setError, setShowAgentSearch, setActiveSession, selectWorktree } from "./store";

const baseName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;
/** full-text search over past Claude/Codex conversations; a hit in a known worktree reopens that conversation */
export function AgentSearch() {
  const [q, setQ] = createSignal("");
  const [hits, setHits] = createSignal<AgentHit[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [done, setDone] = createSignal(false);
  const [sel, setSel] = createSignal(0);
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
        if (mine === seq) setError(String(e).includes("unknown method") ? "Reinicia o daemon para usar a pesquisa de conversas." : String(e));
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
      const id = await launchDefaultAgent(t.project.id, t.wt.path, "claude", "claude", { resume: h.session }, undefined, projectTerse(t.project.id));
      await refreshSessions();
      setActiveSession(id);
    } catch (e) {
      setError(String(e));
    }
  };

  createEffect(() => {
    hits();
    setSel(0);
  });
  // keep the highlighted hit visible while arrowing through a long list
  createEffect(() => {
    sel();
    queueMicrotask(() => document.querySelector(".search-hit.on")?.scrollIntoView({ block: "nearest" }));
  });
  const onKey = (e: KeyboardEvent) => {
    // Home and End stay with the text field (cursor)
    const to = e.key.startsWith("Arrow") ? moveIndex(sel(), hits().length, e.key) : null;
    if (to !== null) {
      e.preventDefault();
      setSel(to);
    } else if (e.key === "Enter" && hits()[sel()]) {
      e.preventDefault();
      void open(hits()[sel()]);
    }
  };

  return (
    <Modal label="Pesquisar conversas" onClose={() => setShowAgentSearch(false)} class="modal auto">
      <input
        class="m-title"
        placeholder="Pesquisar nas conversas do Claude e do Codex…"
        role="combobox"
        aria-expanded={hits().length > 0}
        aria-controls="search-list"
        aria-activedescendant={hits().length ? `search-hit-${sel()}` : undefined}
        aria-label="Pesquisar nas conversas"
        value={q()}
        onKeyDown={onKey}
        onInput={(e) => {
          setQ(e.currentTarget.value);
          run(e.currentTarget.value);
        }}
        ref={(el) => queueMicrotask(() => el.focus())}
      />
      <div class="auto-list" id="search-list" role="listbox" aria-label="Conversas encontradas">
        <For each={hits()}>
          {(h, i) => (
            <div
              class="auto-item search-hit"
              id={`search-hit-${i()}`}
              role="option"
              aria-selected={i() === sel()}
              aria-disabled={!target(h)}
              classList={{ off: !target(h), on: i() === sel() }}
              onMouseEnter={() => setSel(i())}
              onClick={() => void open(h)}
              title={target(h) ? "Retomar esta conversa" : h.cwd}
            >
              <div class="auto-main">
                <div class="auto-name">
                  {h.agent} · {baseName(h.cwd || "?")} <span class="auto-note">{ago(Date.now() / 1000 - h.ts)}</span>
                </div>
                <div class="auto-note">{h.snippet}</div>
              </div>
            </div>
          )}
        </For>
      </div>
      <div class="auto-note" role="status">
        <Show when={busy()}>A pesquisar…</Show>
        <Show when={!busy() && done() && hits().length === 0}>Nada encontrado.</Show>
      </div>
      <div class="auto-note">Só as conversas do Claude em worktrees conhecidos podem ser retomadas aqui; as restantes aparecem a cinzento.</div>
    </Modal>
  );
}
