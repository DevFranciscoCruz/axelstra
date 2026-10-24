import { Badge, Button, Field, Input, Select, Textarea } from "./ui";
import { createPoller } from "./poller";
import { Icon } from "./Icon";
import { ask } from "./Confirm";
import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { AgentInfo, Board, KCard, agentOptions, agentWarning, agentsList, OrchRun, gitPr, gitPush, kanbanDelete, kanbanList, kanbanMove, kanbanSave, orchRetry, orchStart, orchStatus, orchTrustInfo } from "./api";
import { Modal } from "./Modal";
import { refreshProjects, selectWorktree, setError, setShowKanban, showNotice, toast } from "./store";
import { planStart, restorePayload } from "./kanbanLogic";
import { S } from "./strings";
import { P } from "./strings.panels";
import { press } from "./a11y";

/** only cards are accepted by the columns: a terminal tab or a file dragged over is ignored */
const CARD_MIME = "application/x-axelstra-card";

const COLS: { id: string; label: string }[] = ["backlog", "todo", "in-progress", "needs-review", "done"].map((id) => ({ id, label: P.kanban.cols[id] }));

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
    const trust = await orchTrustInfo(props.project).catch(() => null);
    const plan = planStart(n, trust, cmds);
    if (!(await ask(plan.message, { title: P.kanban.runTitle, kind: plan.warn ? "warning" : "info", ...(plan.bypass.length ? { typeToConfirm: "bypass", okLabel: P.kanban.runBypass } : {}) }))) return;
    try {
      setRun(await orchStart(props.project, 3, plan.trustAcceptance, plan.pin));
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
    const t = createPoller(() => draft() === null && Promise.all([reload(), pollRun()]), 3000);
    onCleanup(t.stop);
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
    if (JSON.stringify(d) !== draftBase && !(await ask(P.kanban.discardAsk, { title: P.kanban.discardTitle, kind: "warning", okLabel: P.kanban.discard }))) return;
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
    const card = board().cards.find((c) => c.meta.id === d.id);
    // undo re-creates the card from what was on the board, so only ask up front when that copy is missing
    if (!card && !(await ask(P.kanban.deleteAsk(d.title), { title: P.kanban.deleteTitle, kind: "danger" }))) return;
    try {
      await kanbanDelete(props.project, d.id);
      setDraft(null);
      toast({
        text: S.cardDeleted(d.title),
        kind: "info",
        ...(card ? { action: { label: P.kanban.undo, run: () => void kanbanSave(props.project, restorePayload(card)).then(reload, (e) => setError(String(e))) } } : {}),
      });
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
  const reason = (id: string) => run().needs_review?.find((r) => r[0] === id)?.[1] ?? P.kanban.waitingReview;
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
    if (!wt) return setErr(P.kanban.unknownWt);
    setShowKanban(false);
    void selectWorktree(wt);
  };
  const retry = (c: KCard) =>
    act(async () => {
      const acc = c.meta.acceptance;
      if (acc && !(await ask(P.kanban.retryAsk(c.meta.title || c.meta.id, acc), { title: P.kanban.retryTitle, kind: "warning" }))) return;
      await orchRetry(props.project, c.meta.id, !!acc);
    });
  const openPr = (c: KCard) =>
    act(async () => {
      const wt = taskWt(c.meta.id);
      if (!wt) throw new Error(P.kanban.unknownWtShort);
      if (!(await ask(P.kanban.prAsk(c.meta.title || c.meta.id), { title: P.kanban.prTitle, kind: "warning" }))) return;
      await gitPush(wt);
      setErr(P.kanban.prCreated(await gitPr(wt, c.meta.title || c.meta.id, c.body)));
    });

  return (
    <div class="kanban">
      <div class="kbar">
        <span class="ktitle">{P.kanban.title}</span>
        <span class="grow" />
        <Button
          variant="default"
          class="primary"
          title={P.kanban.runTip}
          disabled={inCol("todo").length === 0 || run().state === "running"}
          onClick={() => void start()}
        >
          {run().state === "running" ? P.kanban.running : P.kanban.runTodo(inCol("todo").length)}
        </Button>
      </div>
      <Show when={err()}>
        <div class="kerr">{err()}</div>
      </Show>
      <Show when={run().state !== "idle"}>
        <div class="krun" classList={{ bad: run().state === "error" || (run().failed?.length ?? 0) > 0 }}>
          <Show when={run().state === "running"}>
            {P.kanban.runningBanner(run().name, run().integ_branch)}
          </Show>
          <Show when={run().state === "done"}>
            {P.kanban.doneBanner(run().merged?.length ?? 0, run().integ_branch)}
            <Show when={(run().failed?.length ?? 0) > 0}>{P.kanban.failedBanner(run().failed!.map((f) => `${f[0]} (${f[1]})`).join(", "))}</Show>
            <Show when={(run().needs_review?.length ?? 0) > 0}>{P.kanban.reviewBanner(run().needs_review!.map((f) => `${f[0]} (${f[1]})`).join(", "))}</Show>
            <Show when={(run().blocked?.length ?? 0) > 0}>{P.kanban.blockedBanner(run().blocked!.join(", "))}</Show>
          </Show>
          <Show when={run().state === "error"}>{P.kanban.errorBanner(run().error)}</Show>
        </div>
      </Show>
      <Show when={board().broken.length > 0}>
        <div class="kerr">
          {P.kanban.broken}{" "}
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
                <Button variant="icon" size="icon" title={P.kanban.newCard} aria-label={P.kanban.newCard} onClick={() => edit(undefined, col.id)}>
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
                        <Badge variant="agent" title={P.kanban.agent}>{c.meta.agent}</Badge>
                        <Show when={c.meta.priority !== 0}>
                          <Badge variant={c.meta.priority > 0 ? "high" : "default"} title={P.kanban.priority}>
                            <Icon name={c.meta.priority > 0 ? "triUp" : "triDown"} size={9} inline /> {Math.abs(c.meta.priority)}
                          </Badge>
                        </Show>
                        <Show when={c.meta.depends_on.length > 0}>
                          <Badge variant="agent" title={P.kanban.dependsOn(c.meta.depends_on.join(", "))}>
                            <Icon name="link" size={11} inline /> {c.meta.depends_on.length}
                          </Badge>
                        </Show>
                        <Badge variant="id" title={P.kanban.fileId}>{c.meta.id}</Badge>
                      </div>
                      <Show when={col.id === "needs-review"}>
                        <div class="kcard-body">{P.kanban.reason(reason(c.meta.id))}</div>
                        <div class="kchips" role="group" aria-label={P.kanban.cardActions} onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
                          <Button size="sm" onClick={() => openWt(c.meta.id)}>{P.kanban.openWt}</Button>
                          <Button size="sm" onClick={() => void retry(c)}>{P.kanban.retry}</Button>
                          <Button size="sm" onClick={() => void openPr(c)}>{P.kanban.openPr}</Button>
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
                          {P.kanban.moveTo}
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
          <Modal label={d().id ? P.kanban.editCard : P.kanban.newCard} dirty onClose={() => void closeDraft()}>
              <div class="m-heading">{d().id ? P.kanban.editCard : P.kanban.newCard}</div>
              <Input
                class="text-lg"
                placeholder={P.kanban.titlePlaceholder}
                value={d().title}
                onInput={(e) => setDraft({ ...d(), title: e.currentTarget.value })}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
              <Textarea
                class="min-h-40 flex-1 font-mono"
                placeholder={P.kanban.bodyPlaceholder}
                value={d().body}
                onInput={(e) => setDraft({ ...d(), body: e.currentTarget.value })}
              />
              <div class="m-row">
                <Field label={P.kanban.status}>
                  <Select
                    value={d().status}
                    options={COLS.map((c) => ({ value: c.id, label: c.label }))}
                    onChange={(v) => setDraft({ ...d(), status: v })}
                  />
                </Field>
                <Field label={P.kanban.agent} hint={agentWarning(agents(), d().agent) || undefined}>
                  <Select
                    value={d().agent}
                    iconOnly
                  options={agentOptions(agents(), d().agent)}
                    onChange={(v) => setDraft({ ...d(), agent: v })}
                  />
                </Field>
                <Field label={P.kanban.priority}>
                  <Input
                    type="number"
                    value={d().priority}
                    onInput={(e) => setDraft({ ...d(), priority: parseInt(e.currentTarget.value) || 0 })}
                  />
                </Field>
              </div>
              <Field label={P.kanban.depends} full>
                <Input value={d().depends} onInput={(e) => setDraft({ ...d(), depends: e.currentTarget.value })} />
              </Field>
              <div class="m-actions">
                <Show when={d().id}>
                  <Button variant="destructive" onClick={() => void remove()}>
                    {P.kanban.delete}
                  </Button>
                </Show>
                <span class="grow" />
                <Button onClick={() => void closeDraft()}>{P.kanban.cancel}</Button>
                <Button variant="default" disabled={!d().title.trim()} onClick={() => void save()}>
                  {P.kanban.save}
                </Button>
              </div>
          </Modal>
        )}
      </Show>
    </div>
  );
}
