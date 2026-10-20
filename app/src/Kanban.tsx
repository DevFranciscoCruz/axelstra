import { ask } from "@tauri-apps/plugin-dialog";
import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { Board, KCard, kanbanDelete, kanbanList, kanbanMove, kanbanSave } from "./api";

const COLS: { id: string; label: string }[] = [
  { id: "backlog", label: "Ideias" },
  { id: "todo", label: "Para fazer" },
  { id: "in-progress", label: "Em curso" },
  { id: "needs-review", label: "Rever" },
  { id: "done", label: "Feito" },
];

type Draft = { id?: string; title: string; body: string; status: string; depends: string; agent: string; priority: number };

export function Kanban(props: { project: string; onRun?: () => void }) {
  const [board, setBoard] = createSignal<Board>({ cards: [], broken: [] });
  const [err, setErr] = createSignal("");
  const [draft, setDraft] = createSignal<Draft | null>(null);
  const [over, setOver] = createSignal<string | null>(null);

  const reload = async () => {
    try {
      setBoard(await kanbanList(props.project));
      setErr("");
    } catch (e) {
      setErr(String(e));
    }
  };
  onMount(() => {
    void reload();
    // agents move cards while the orchestrator runs
    const t = setInterval(() => !document.hidden && draft() === null && void reload(), 3000);
    onCleanup(() => clearInterval(t));
  });

  const inCol = (id: string) => board().cards.filter((c) => c.meta.status === id);
  // cards with an unknown status would vanish: show them under "Ideias"
  const known = new Set(COLS.map((c) => c.id));
  const colCards = (id: string) =>
    id === "backlog" ? board().cards.filter((c) => !known.has(c.meta.status) || c.meta.status === "backlog") : inCol(id);

  const edit = (c?: KCard, status = "backlog") =>
    setDraft(
      c
        ? { id: c.meta.id, title: c.meta.title, body: c.body, status: c.meta.status, depends: c.meta.depends_on.join(", "), agent: c.meta.agent, priority: c.meta.priority }
        : { title: "", body: "", status, depends: "", agent: "claude", priority: 0 },
    );

  const save = async () => {
    const d = draft();
    if (!d || !d.title.trim()) return;
    try {
      await kanbanSave(props.project, {
        id: d.id,
        title: d.title.trim(),
        body: d.body,
        status: d.status,
        agent: d.agent,
        priority: d.priority,
        depends_on: d.depends.split(/[\s,]+/).filter(Boolean),
      });
      setDraft(null);
      await reload();
    } catch (e) {
      setErr(String(e));
    }
  };

  const remove = async () => {
    const d = draft();
    if (!d?.id) return;
    if (!(await ask(`Apagar o cartão "${d.title}"? O ficheiro é removido.`, { title: "Apagar", kind: "warning" }))) return;
    try {
      await kanbanDelete(props.project, d.id);
      setDraft(null);
      await reload();
    } catch (e) {
      setErr(String(e));
    }
  };

  const drop = async (status: string, e: DragEvent) => {
    e.preventDefault();
    setOver(null);
    const id = e.dataTransfer?.getData("text/plain");
    if (!id) return;
    try {
      await kanbanMove(props.project, id, status);
    } catch (x) {
      setErr(String(x));
    }
    await reload();
  };

  return (
    <div class="kanban">
      <div class="kbar">
        <span class="ktitle">Kanban</span>
        <span class="grow" />
        <Show when={props.onRun}>
          <button
            class="primary"
            title="Cria um worktree de integração e despacha um agente por cartão em 'Para fazer'"
            disabled={inCol("todo").length === 0}
            onClick={() => props.onRun?.()}
          >
            Executar “Para fazer” ({inCol("todo").length})
          </button>
        </Show>
      </div>
      <Show when={err()}>
        <div class="kerr">{err()}</div>
      </Show>
      <Show when={board().broken.length > 0}>
        <div class="kerr">
          Cartões ilegíveis (corrija o ficheiro em .kanban):{" "}
          <For each={board().broken}>{(b) => <span title={b.error}>{b.file} </span>}</For>
        </div>
      </Show>
      <div class="kcols">
        <For each={COLS}>
          {(col) => (
            <div
              class="kcol"
              classList={{ over: over() === col.id }}
              onDragOver={(e) => {
                e.preventDefault();
                setOver(col.id);
              }}
              onDragLeave={() => setOver(null)}
              onDrop={(e) => void drop(col.id, e)}
            >
              <div class="kcol-head">
                <span>{col.label}</span>
                <span class="kcount">{colCards(col.id).length}</span>
                <button class="icon" title="Novo cartão" onClick={() => edit(undefined, col.id)}>
                  +
                </button>
              </div>
              <div class="kcards">
                <For each={colCards(col.id)}>
                  {(c) => (
                    <div
                      class="kcard"
                      draggable={true}
                      onDragStart={(e) => e.dataTransfer?.setData("text/plain", c.meta.id)}
                      onClick={() => edit(c)}
                    >
                      <div class="kcard-title">{c.meta.title || c.meta.id}</div>
                      <Show when={c.body.trim()}>
                        <div class="kcard-body">{c.body.trim().slice(0, 140)}</div>
                      </Show>
                      <Show when={c.meta.depends_on.length > 0}>
                        <div class="kcard-meta">depende de {c.meta.depends_on.join(", ")}</div>
                      </Show>
                    </div>
                  )}
                </For>
              </div>
            </div>
          )}
        </For>
      </div>

      <Show when={draft()}>
        {(d) => (
          <div class="modal-back" onClick={() => setDraft(null)}>
            <div class="modal" onClick={(e) => e.stopPropagation()}>
              <input
                class="m-title"
                placeholder="Título"
                value={d().title}
                onInput={(e) => setDraft({ ...d(), title: e.currentTarget.value })}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
              <textarea
                class="m-body"
                placeholder="Escreva livremente: o contexto, o que quer, critérios… O agente lê isto."
                value={d().body}
                onInput={(e) => setDraft({ ...d(), body: e.currentTarget.value })}
              />
              <div class="m-row">
                <label>
                  Estado
                  <select value={d().status} onChange={(e) => setDraft({ ...d(), status: e.currentTarget.value })}>
                    <For each={COLS}>{(c) => <option value={c.id}>{c.label}</option>}</For>
                  </select>
                </label>
                <label>
                  Agente
                  <input value={d().agent} onInput={(e) => setDraft({ ...d(), agent: e.currentTarget.value })} />
                </label>
                <label>
                  Prioridade
                  <input
                    type="number"
                    value={d().priority}
                    onInput={(e) => setDraft({ ...d(), priority: parseInt(e.currentTarget.value) || 0 })}
                  />
                </label>
              </div>
              <label class="m-full">
                Depende de (ids separados por vírgula)
                <input value={d().depends} onInput={(e) => setDraft({ ...d(), depends: e.currentTarget.value })} />
              </label>
              <div class="m-actions">
                <Show when={d().id}>
                  <button class="danger" onClick={() => void remove()}>
                    Apagar
                  </button>
                </Show>
                <span class="grow" />
                <button onClick={() => setDraft(null)}>Cancelar</button>
                <button class="primary" disabled={!d().title.trim()} onClick={() => void save()}>
                  Guardar
                </button>
              </div>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
}
