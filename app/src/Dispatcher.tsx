import { For, Show, createMemo, onCleanup } from "solid-js";
import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { StateDot } from "./StateDot";
import { dispatchTopics, norm, projects, selectWorktree, sessionState, sessionsOf, setShowDispatcher, watchDispatch } from "./store";

/** Live view of what the main session dispatched: one card per topic (worktree) with its latest requests. */
export function Dispatcher() {
  // the store polls once for everyone who shows the topics (and not while the window is hidden)
  onCleanup(watchDispatch());
  const meta = dispatchTopics;

  const topics = createMemo(() => {
    const out: { project: string; path: string; branch: string; title: string; requests: string[]; archived: boolean; updated: number }[] = [];
    for (const p of projects()) {
      for (const w of p.worktrees) {
        const m = meta()[norm(w.path)];
        if (!m || w.is_main) continue;
        out.push({ project: p.name, path: w.path, branch: w.branch ?? "(detached)", title: m.title ?? w.branch ?? w.path, requests: m.requests ?? [], archived: !!m.archived, updated: m.updated ?? 0 });
      }
    }
    return out.sort((a, b) => Number(a.archived) - Number(b.archived) || b.updated - a.updated);
  });

  const stateOf = (path: string) => {
    const agents = sessionsOf(path).filter((s) => s.meta?.kind === "agent");
    if (agents.some((s) => sessionState(s) === "waiting")) return { dot: "waiting" as const, text: "à espera de resposta" };
    if (agents.some((s) => sessionState(s) === "working")) return { dot: "working" as const, text: "a trabalhar" };
    if (agents.some((s) => s.running)) return { dot: "idle" as const, text: "parado" };
    return { dot: "none" as const, text: "sem agente" };
  };

  return (
    <Modal label="Despachante" class="modal settings" onClose={() => setShowDispatcher(false)}>
      <div class="auto-head">
        <strong>Despachante</strong>
        <span class="grow" />
        <button onClick={() => setShowDispatcher(false)}>Fechar</button>
      </div>
      <p class="auto-note">Cada pedido feito na main vai para o worktree do seu tópico. Aqui vê para onde foi cada um.</p>
      <div class="disp-list">
        <For each={topics()} fallback={<div class="auto-note">Ainda não há pedidos despachados. Peça algo ao agente da main.</div>}>
          {(t) => {
            const st = () => stateOf(t.path);
            return (
              <div class="disp-card" classList={{ archived: t.archived }}>
                <div class="disp-head">
                  <StateDot state={st().dot} />
                  <strong class="disp-title">{t.title}</strong>
                  <span class="auto-note">{t.archived ? "arquivado" : st().text}</span>
                  <span class="grow" />
                  <button
                    class="icon"
                    title="Abrir este worktree"
                    aria-label="Abrir este worktree"
                    onClick={() => {
                      selectWorktree(t.path);
                      setShowDispatcher(false);
                    }}
                  >
                    <Icon name="external" />
                  </button>
                </div>
                <div class="auto-note">{t.project} · {t.branch}</div>
                <Show when={t.requests.length > 0}>
                  <ol class="disp-reqs">
                    <For each={t.requests.slice(-3).reverse()}>{(r) => <li>{r}</li>}</For>
                  </ol>
                </Show>
              </div>
            );
          }}
        </For>
      </div>
    </Modal>
  );
}
