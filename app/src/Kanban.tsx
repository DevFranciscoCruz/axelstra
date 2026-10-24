import { Badge, Button, Field, Input, Textarea } from "./ui";
import { Select } from "./Select";
import { Icon } from "./Icon";
import { ask } from "./Confirm";
import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { AgentInfo, Board, KCard, agentOptions, agentWarning, agentsList, OrchRun, gitPr, gitPush, kanbanDelete, kanbanList, kanbanMove, kanbanSave, orchRetry, orchStart, orchStatus } from "./api";
import { Modal } from "./Modal";
import { refreshProjects, selectWorktree, setError, setShowKanban, showNotice } from "./store";
import { S } from "./strings";
import { press } from "./a11y";

/** only cards are accepted by the columns: a terminal tab or a file dragged over is ignored */
const CARD_MIME = "application/x-axelstra-card";

const COLS: { id: string; label: string }[] = [
  { id: "backlog", label: "Ideias" },
  { id: "todo", label: "Para fazer" },
  { id: "in-progress", label: "Em curso" },
  { id: "needs-review", label: "Rever" },
  { id: "done", label: "Feito" },
];

type Draft = { id?: string; title: string; body: string; status: string; depends: string; agent: string; priority: number };

export function Kanban(props: { project: string }) {
  const [board, setBoard] = createSignal<Board>({ cards: [], broken: [] });
  const [err, setErr] = createSignal("");
  const [agents, setAgents] = createSignal<AgentInfo[]>([]);
  const [draft, setDraft] = createSignal<Draft | null>(null);
  const [over, setOver] = createSignal<string | null>(null);
  const [dragId, setDragId] = createSignal<string | null>(null);
  const [menu, setMenu] = createSignal<string | null>(null);
  let draftBase = "";
  const [run, setRun] = createSignal<OrchRun>({ state: "idle" });

  const pollRun = async () => {
    try {
      const prev = run().state;
      const r = await orchStatus(props.project);
      setRun(r);
      // the integration worktree appears in the sidebar once the run starts
      if (r.state !== prev) void refreshProjects();
    } catch {
      /* daemon briefly unavailable: the next tick retries */
    }
  };
  const start = async () => {
    const n = inCol("todo").length;
    // acceptance commands run in a shell: a card from a cloned repo could carry anything
    const cmds = inCol("todo").filter((c) => c.meta.acceptance).map((c) => `${c.meta.id}: ${c.meta.acceptance}`);
    if (
      !(await ask(
        `Despachar ${n} agente(s) para os cartões em "Para fazer"? Cada um corre num worktree isolado e o resultado é integrado num branch novo (orch/…). Nada é enviado para o remote.` +
          (cmds.length ? `\n\nEstes comandos de aceitação vão correr na sua máquina:\n${cmds.join("\n")}\n\nSó confirme se confia nestes cartões.` : ""),
        { title: "Executar orquestração", kind: cmds.length ? "warning" : "info" },
      ))
    )
      return;
    try {
      setRun(await orchStart(props.project, 3, cmds.length > 0));
      void refreshProjects();
    } catch (e) {
      setErr(String(e));
    }
  };

  // only the newest answer counts: the 3s poll and a manual reload can overlap
  let reloadReq = 0;
  const reload = async () => {
    const r = ++reloadReq;
    try {
      const b = await kanbanList(props.project);
      if (r !== reloadReq) return;
      setBoard(b);
      setErr("");
    } catch (e) {
      if (r === reloadReq) setErr(String(e));
    }
  };
  onMount(() => {
    agentsList().then(setAgents, () => {});
    void reload();
    void pollRun();
    // agents move cards while the orchestrator runs
    const t = setInterval(() => !document.hidden && draft() === null && (void reload(), void pollRun()), 3000);
    onCleanup(() => clearInterval(t));
  });

  const inCol = (id: string) => board().cards.filter((c) => c.meta.status === id);
  // cards with an unknown status would vanish: show them under "Ideias"
  const known = new Set(COLS.map((c) => c.id));
  const colCards = (id: string) =>
    id === "backlog" ? board().cards.filter((c) => !known.has(c.meta.status) || c.meta.status === "backlog") : inCol(id);

  const edit = (c?: KCard, status = "backlog") => {
    setDraft(
      c
        ? { id: c.meta.id, title: c.meta.title, body: c.body, status: c.meta.status, depends: c.meta.depends_on.join(", "), agent: c.meta.agent, priority: c.meta.priority }
        : { title: "", body: "", status, depends: "", agent: "claude", priority: 0 },
    );
    draftBase = JSON.stringify(draft());
  };
  // Esc, Cancel and the backdrop all end here: a draft with edits asks first
  const closeDraft = async () => {
    const d = draft();
    if (!d) return;
    if (JSON.stringify(d) !== draftBase && !(await ask("Descartar as alterações a este cartão?", { title: "Descartar", kind: "warning", okLabel: "Descartar" }))) return;
    setDraft(null);
  };

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
    if (!(await ask(`Apagar o cartão "${d.title}"? O ficheiro é removido.`, { title: "Apagar", kind: "danger" }))) return;
    try {
      await kanbanDelete(props.project, d.id);
      setDraft(null);
      showNotice(S.cardDeleted(d.title));
      await reload();
    } catch (e) {
      setError(S.deleteFailed(d.title, e));
    }
  };

  const drop = async (status: string, e: DragEvent) => {
    e.preventDefault();
    setOver(null);
    setDragId(null);
    const id = e.dataTransfer?.getData(CARD_MIME);
    if (!id) return;
    await moveTo(id, status);
  };

  const moveTo = async (id: string, status: string) => {
    setMenu(null);
    try {
      await kanbanMove(props.project, id, status);
    } catch (x) {
      setErr(String(x));
    }
    await reload();
  };

  // a needs-review card keeps its task worktree: it sits next to the integration one
  const taskWt = (id: string) => {
    const r = run();
    if (!r.integ_wt || !r.name) return null;
    const i = Math.max(r.integ_wt.lastIndexOf("/"), r.integ_wt.lastIndexOf("\\"));
    return `${r.integ_wt.slice(0, i + 1)}${r.name}-${id}`;
  };
  const reason = (id: string) => run().needs_review?.find((r) => r[0] === id)?.[1] ?? "aguarda revisão";
  const act = async (fn: () => Promise<unknown>) => {
    try {
      await fn();
    } catch (x) {
      setErr(String(x));
    }
    await reload();
    await pollRun();
  };
  const openWt = (id: string) => {
    const wt = taskWt(id);
    if (!wt) return setErr("O worktree desta execução já não é conhecido: inicie a orquestração de novo.");
    setShowKanban(false);
    void selectWorktree(wt);
  };
  const retry = (c: KCard) =>
    act(async () => {
      const acc = c.meta.acceptance;
      if (acc && !(await ask(`Integrar "${c.meta.title || c.meta.id}" outra vez? Corre o comando de aceitação na sua máquina:\n${acc}`, { title: "Integrar de novo", kind: "warning" }))) return;
      await orchRetry(props.project, c.meta.id, !!acc);
    });
  const openPr = (c: KCard) =>
    act(async () => {
      const wt = taskWt(c.meta.id);
      if (!wt) throw new Error("worktree desconhecido");
      if (!(await ask(`Enviar o branch de "${c.meta.title || c.meta.id}" e criar um pull request no GitHub?`, { title: "Pull request", kind: "warning" }))) return;
      await gitPush(wt);
      setErr(`Pull request criado: ${await gitPr(wt, c.meta.title || c.meta.id, c.body)}`);
    });

  return (
    <div class="kanban">
      <div class="kbar">
        <span class="ktitle">Kanban</span>
        <span class="grow" />
        <Button
          variant="default"
          class="primary"
          title="Cria um worktree de integração e despacha um agente por cartão em 'Para fazer'"
          disabled={inCol("todo").length === 0 || run().state === "running"}
          onClick={() => void start()}
        >
          {run().state === "running" ? "A executar…" : `Executar “Para fazer” (${inCol("todo").length})`}
        </Button>
      </div>
      <Show when={err()}>
        <div class="kerr">{err()}</div>
      </Show>
      <Show when={run().state !== "idle"}>
        <div class="krun" classList={{ bad: run().state === "error" || (run().failed?.length ?? 0) > 0 }}>
          <Show when={run().state === "running"}>
            A executar {run().name} – branch {run().integ_branch}. Os agentes aparecem como terminais nos worktrees task.
          </Show>
          <Show when={run().state === "done"}>
            Terminou: {run().merged?.length ?? 0} integrados em {run().integ_branch}
            <Show when={(run().failed?.length ?? 0) > 0}> · falharam: {run().failed!.map((f) => `${f[0]} (${f[1]})`).join(", ")}</Show>
            <Show when={(run().needs_review?.length ?? 0) > 0}> · rever: {run().needs_review!.map((f) => `${f[0]} (${f[1]})`).join(", ")}</Show>
            <Show when={(run().blocked?.length ?? 0) > 0}> · bloqueados: {run().blocked!.join(", ")}</Show>
          </Show>
          <Show when={run().state === "error"}>Orquestração falhou: {run().error}</Show>
        </div>
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
                if (!e.dataTransfer?.types.includes(CARD_MIME)) return;
                e.preventDefault();
                setOver(col.id);
              }}
              onDragLeave={(e) => !e.currentTarget.contains(e.relatedTarget as Node | null) && setOver(null)}
              onDrop={(e) => void drop(col.id, e)}
            >
              <div class="kcol-head">
                <span>{col.label}</span>
                <span class="kcount">{colCards(col.id).length}</span>
                <Button variant="icon" size="icon" title="Novo cartão" aria-label="Novo cartão" onClick={() => edit(undefined, col.id)}>
                  <Icon name="plus" />
                </Button>
              </div>
              <div class="kcards">
                <For each={colCards(col.id)}>
                  {(c) => (
                    <div
                      class={`kcard st-${col.id}`}
                      classList={{ dragging: dragId() === c.meta.id }}
                      draggable={true}
                      onDragStart={(e) => {
                        e.dataTransfer?.setData(CARD_MIME, c.meta.id);
                        setDragId(c.meta.id);
                      }}
                      onDragEnd={() => setDragId(null)}
                      {...press(() => edit(c))}
                    >
                      <div class="kcard-title">{c.meta.title || c.meta.id}</div>
                      <Show when={c.body.trim()}>
                        <div class="kcard-body">{c.body.trim().slice(0, 160)}</div>
                      </Show>
                      <div class="kchips">
                        <Badge variant="agent" title="Agente">{c.meta.agent}</Badge>
                        <Show when={c.meta.priority !== 0}>
                          <Badge variant={c.meta.priority > 0 ? "high" : "default"} title="Prioridade">
                            <Icon name={c.meta.priority > 0 ? "triUp" : "triDown"} size={9} inline /> {Math.abs(c.meta.priority)}
                          </Badge>
                        </Show>
                        <Show when={c.meta.depends_on.length > 0}>
                          <Badge variant="agent" title={`Depende de ${c.meta.depends_on.join(", ")}`}>
                            <Icon name="link" size={11} inline /> {c.meta.depends_on.length}
                          </Badge>
                        </Show>
                        <Badge variant="id" title="Id do ficheiro .kanban">{c.meta.id}</Badge>
                      </div>
                      <Show when={col.id === "needs-review"}>
                        <div class="kcard-body">Motivo: {reason(c.meta.id)}</div>
                        <div class="kchips" role="group" aria-label="Ações do cartão" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
                          <Button size="sm" onClick={() => openWt(c.meta.id)}>Abrir worktree</Button>
                          <Button size="sm" onClick={() => void retry(c)}>Tentar integrar de novo</Button>
                          <Button size="sm" onClick={() => void openPr(c)}>Abrir pull request</Button>
                        </div>
                      </Show>
                      <div
                        role="group" style={{ position: "relative" }}
                        onClick={(e) => e.stopPropagation()}
                        onFocusOut={(e) => !e.currentTarget.contains(e.relatedTarget as Node | null) && menu() === c.meta.id && setMenu(null)}
                        onKeyDown={(e) => {
                          e.stopPropagation();
                          if (e.key === "Escape" && menu() === c.meta.id) {
                            setMenu(null);
                            e.currentTarget.querySelector("button")?.focus();
                          }
                        }}
                      >
                        <Button size="sm" aria-haspopup="menu" aria-expanded={menu() === c.meta.id} onClick={() => setMenu(menu() === c.meta.id ? null : c.meta.id)}>
                          Mover para…
                        </Button>
                        <Show when={menu() === c.meta.id}>
                          <div role="menu" style={{ display: "flex", "flex-direction": "column", gap: "2px", "margin-top": "4px" }}>
                            <For each={COLS.filter((x) => x.id !== col.id)}>
                              {(x, i) => (
                                <Button size="sm" class="justify-start" role="menuitem" ref={(el) => queueMicrotask(() => i() === 0 && el.focus())} onClick={() => void moveTo(c.meta.id, x.id)}>
                                  {x.label}
                                </Button>
                              )}
                            </For>
                          </div>
                        </Show>
                      </div>
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
          <Modal label={d().id ? "Editar cartão" : "Novo cartão"} dirty onClose={() => void closeDraft()}>
              <div class="m-heading">{d().id ? "Editar cartão" : "Novo cartão"}</div>
              <Input
                class="text-lg"
                placeholder="Título"
                value={d().title}
                onInput={(e) => setDraft({ ...d(), title: e.currentTarget.value })}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
              <Textarea
                class="min-h-40 flex-1 font-mono"
                placeholder="Escreve livremente: o contexto, o que queres, critérios… O agente lê isto."
                value={d().body}
                onInput={(e) => setDraft({ ...d(), body: e.currentTarget.value })}
              />
              <div class="m-row">
                <Field label="Estado">
                  <Select
                    value={d().status}
                    options={COLS.map((c) => ({ value: c.id, label: c.label }))}
                    onChange={(v) => setDraft({ ...d(), status: v })}
                  />
                </Field>
                <Field label="Agente" hint={agentWarning(agents(), d().agent) || undefined}>
                  <Select
                    value={d().agent}
                    iconOnly
                  options={agentOptions(agents(), d().agent)}
                    onChange={(v) => setDraft({ ...d(), agent: v })}
                  />
                </Field>
                <Field label="Prioridade">
                  <Input
                    type="number"
                    value={d().priority}
                    onInput={(e) => setDraft({ ...d(), priority: parseInt(e.currentTarget.value) || 0 })}
                  />
                </Field>
              </div>
              <Field label="Depende de (ids separados por vírgula)" full>
                <Input value={d().depends} onInput={(e) => setDraft({ ...d(), depends: e.currentTarget.value })} />
              </Field>
              <div class="m-actions">
                <Show when={d().id}>
                  <Button variant="destructive" onClick={() => void remove()}>
                    Apagar
                  </Button>
                </Show>
                <span class="grow" />
                <Button onClick={() => void closeDraft()}>Cancelar</Button>
                <Button variant="default" disabled={!d().title.trim()} onClick={() => void save()}>
                  Guardar
                </Button>
              </div>
          </Modal>
        )}
      </Show>
    </div>
  );
}
