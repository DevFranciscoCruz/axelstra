import { Badge, Button, Field, Input, Select, Textarea } from "./ui";
import { createLive } from "./live";
import { Icon } from "./Icon";
import { ask } from "./Confirm";
import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { AgentInfo, Board, KCard, agentOptions, agentWarning, agentsList, OrchRun, gitPr, gitPush, kanbanDelete, kanbanList, kanbanMove, kanbanSave, orchRetry, orchStart, orchStatus, orchTrustInfo } from "./api";
import { Modal } from "./Modal";
import { refreshProjects, selectWorktree, setError, setShowKanban, toast } from "./store";
import { COL_IDS, applyPlan, colAt, columnCards, edgeDelta, indexAt, keyStep, planMove, planStart, posOf, restorePayload, type DropPos } from "./kanbanLogic";
import { S } from "./strings";
import { P } from "./strings.panels";
import { coded } from "./msg";

const COLS: { id: string; label: string }[] = COL_IDS.map((id) => ({ id, label: P.kanban.cols[id] }));

/** a pointer drag starts after this many pixels, so a plain click still opens the card */
const DRAG_SLOP = 5;

/** `col` is "" while the pointer is outside every column (dropping there cancels) */
type Drag = DropPos & { id: string; mode: "ptr" | "kbd" };

type Draft = { id?: string; title: string; body: string; status: string; depends: string; agent: string; priority: number };

export function Kanban(props: { project: string }) {
  const [board, setBoard] = createSignal<Board>({ cards: [], broken: [] });
  const [err, setErr] = createSignal("");
  const [agents, setAgents] = createSignal<AgentInfo[]>([]);
  const [draft, setDraft] = createSignal<Draft | null>(null);
  const [drag, setDrag] = createSignal<Drag | null>(null);
  const [live, setLive] = createSignal("");
  let colsEl: HTMLDivElement | undefined;
  let ghostEl: HTMLDivElement | undefined;
  let moving = false;
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
      /* daemon briefly unavailable: the next event or focus retries */
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

  // only the newest answer counts: the poll and a manual reload can overlap
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
  // agents move cards while the orchestrator runs: the daemon says so (`kanban.changed`, `orch.changed`); a board being edited
  // or dragged is left alone and caught up when that ends
  let stale = false;
  const sync = async () => {
    if (draft() !== null || moving || drag() !== null) {
      stale = true;
      return;
    }
    stale = false;
    await Promise.all([reload(), pollRun()]);
  };
  onMount(() => {
    agentsList().then(setAgents, () => {});
    void reload();
    void pollRun();
    const mine = (d: { project?: string }) => !d.project || d.project === props.project;
    const live = createLive(sync, ["kanban.changed", "orch.changed"], { match: mine });
    createEffect(() => {
      if (draft() === null && drag() === null && stale) live.now();
    });
    onCleanup(live.stop);
  });

  const inCol = (id: string) => board().cards.filter((c) => c.meta.status === id);
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

  // cards with an unknown status would vanish: columnCards shows them under "Ideias"
  const colLabel = (id: string) => COLS.find((c) => c.id === id)?.label ?? id;
  const colCards = (id: string) => columnCards(board().cards, id);
  const cardName = (id: string) => {
    const c = board().cards.find((x) => x.meta.id === id);
    return c?.meta.title || id;
  };
  /** the column's cards without the dragged one: `index` counts in this list */
  const without = (col: string, id: string) => colCards(col).filter((c) => c.meta.id !== id);

  const moveTo = async (id: string, status: string) => {
    try {
      await kanbanMove(props.project, id, status);
      return true;
    } catch (x) {
      setErr(String(x));
      return false;
    }
  };

  /** status through kanbanMove (as before), order through the card's `priority` (see kanbanLogic), then one reload */
  const commit = async (id: string, pos: DropPos, kbd: boolean) => {
    const cards = board().cards;
    const plan = planMove(cards, id, pos);
    const total = without(pos.col, id).length + 1;
    setLive(P.kanban.dropped(cardName(id), colLabel(pos.col), pos.index + 1, total));
    if (!plan) return;
    moving = true;
    setBoard({ ...board(), cards: applyPlan(cards, id, plan) });
    try {
      if (!plan.status || (await moveTo(id, plan.status))) {
        for (const p of plan.priorities) {
          const c = cards.find((x) => x.meta.id === p.id);
          if (c) await kanbanSave(props.project, { id: c.meta.id, title: c.meta.title, body: c.body, priority: p.priority });
        }
      }
    } catch (x) {
      setErr(String(x));
    }
    moving = false;
    await reload();
    // the card was re-created by the reload: give the keyboard user their place back
    if (kbd) requestAnimationFrame(() => colsEl?.querySelector<HTMLElement>(`[data-id="${CSS.escape(id)}"]`)?.focus());
  };

  const cancelDrag = () => {
    const d = drag();
    if (d) setLive(P.kanban.dragCancelled(cardName(d.id)));
    setDrag(null);
    endPointer();
  };

  // ---- pointer drag (no HTML5 DnD: WebView2 handles it poorly) ----
  let down: { id: string; x: number; y: number; dx: number; dy: number; w: number } | null = null;
  let px = 0;
  let py = 0;
  let raf = 0;
  let suppressClick = false;

  const colEls = () => Array.from(colsEl?.querySelectorAll<HTMLElement>(".kcol") ?? []);
  const colRects = () =>
    colEls().map((e) => {
      const r = e.getBoundingClientRect();
      return [r.left, r.right] as [number, number];
    });
  /** column and slot under a point; null outside the columns */
  const hit = (x: number, y: number, id: string): DropPos | null => {
    const b = colsEl?.getBoundingClientRect();
    if (!b || y < b.top || y > b.bottom) return null;
    const ci = colAt(colRects(), x);
    if (ci === null) return null;
    const mids = Array.from(colEls()[ci].querySelectorAll<HTMLElement>(".kcard"))
      .filter((e) => e.dataset.id !== id)
      .map((e) => {
        const r = e.getBoundingClientRect();
        return r.top + r.height / 2;
      });
    return { col: COLS[ci].id, index: indexAt(mids, y) };
  };
  const place = () => {
    if (!ghostEl || !down) return;
    ghostEl.style.setProperty("--gx", `${px - down.dx}px`);
    ghostEl.style.setProperty("--gy", `${py - down.dy}px`);
  };
  const retarget = () => {
    const d = drag();
    if (!d || d.mode !== "ptr") return;
    const t = hit(px, py, d.id) ?? { col: "", index: 0 };
    if (t.col !== d.col || t.index !== d.index) setDrag({ ...d, ...t });
  };
  const tick = () => {
    raf = 0;
    const d = drag();
    if (!d || d.mode !== "ptr" || !colsEl) return;
    const b = colsEl.getBoundingClientRect();
    const dx = edgeDelta(px, b.left, b.right);
    if (dx) colsEl.scrollLeft += dx;
    const ci = colAt(colRects(), px);
    const cards = ci === null ? null : colEls()[ci].querySelector<HTMLElement>(".kcards");
    if (cards) {
      const r = cards.getBoundingClientRect();
      const dy = edgeDelta(py, r.top, r.bottom);
      if (dy) cards.scrollTop += dy;
    }
    retarget();
    raf = requestAnimationFrame(tick);
  };
  const onMove = (e: PointerEvent) => {
    if (!down) return;
    px = e.clientX;
    py = e.clientY;
    if (!drag()) {
      if (Math.hypot(px - down.x, py - down.y) < DRAG_SLOP) return;
      setDrag({ id: down.id, col: "", index: 0, mode: "ptr" });
      suppressClick = true;
      raf = requestAnimationFrame(tick);
    }
    place();
    retarget();
  };
  const onUp = () => {
    const d = drag();
    const id = down?.id;
    endPointer();
    if (!d || !id) return;
    setDrag(null);
    if (d.col) void commit(id, { col: d.col, index: d.index }, false);
    else setLive(P.kanban.dragCancelled(cardName(id)));
  };
  const onEsc = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || drag()?.mode !== "ptr") return;
    e.preventDefault();
    e.stopPropagation();
    cancelDrag();
  };
  function endPointer() {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    window.removeEventListener("pointercancel", cancelDrag);
    window.removeEventListener("keydown", onEsc, true);
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
    down = null;
    // the click that ends a drag must not open the card
    setTimeout(() => (suppressClick = false), 50);
  }
  onCleanup(endPointer);
  const onDown = (e: PointerEvent & { currentTarget: HTMLElement }, c: KCard) => {
    if (e.button !== 0 || drag() || (e.target as HTMLElement).closest("button,a,input,textarea,select")) return;
    const r = e.currentTarget.getBoundingClientRect();
    down = { id: c.meta.id, x: e.clientX, y: e.clientY, dx: e.clientX - r.left, dy: e.clientY - r.top, w: r.width };
    px = e.clientX;
    py = e.clientY;
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", cancelDrag);
    window.addEventListener("keydown", onEsc, true);
  };

  // ---- keyboard drag: Space picks up, arrows move, Space drops, Esc cancels ----
  const announcePos = (d: DropPos, id: string) => P.kanban.movedTo(colLabel(d.col), d.index + 1, without(d.col, id).length + 1);
  const onCardKey = (e: KeyboardEvent & { currentTarget: HTMLElement }, c: KCard) => {
    if (e.target !== e.currentTarget || e.ctrlKey || e.metaKey || e.altKey) return;
    const id = c.meta.id;
    const d = drag();
    if (d?.mode === "kbd" && d.id === id) {
      if (e.key === " " || e.key === "Enter") {
        e.preventDefault();
        setDrag(null);
        void commit(id, d, true);
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        cancelDrag();
      } else if (["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) {
        e.preventDefault();
        const n = keyStep(d, e.key, COL_IDS, COL_IDS.map((k) => without(k, id).length));
        setDrag({ ...d, ...n });
        setLive(announcePos(n, id));
      }
      return;
    }
    if (d) return;
    if (e.key === " ") {
      e.preventDefault();
      const p = posOf(board().cards, id);
      if (!p) return;
      setDrag({ id, ...p, mode: "kbd" });
      setLive(P.kanban.grabbed(cardName(id), colLabel(p.col), p.index + 1, without(p.col, id).length + 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      edit(c);
    }
  };
  const dropBefore = (col: string, id: string) => {
    const d = drag();
    return !!d && d.col === col && without(col, d.id)[d.index]?.meta.id === id;
  };
  const dropEnd = (col: string) => {
    const d = drag();
    return !!d && d.col === col && d.index >= without(col, d.id).length;
  };

  // a needs-review card keeps its task worktree: it sits next to the integration one
  const taskWt = (id: string) => {
    const r = run();
    if (!r.integ_wt || !r.name) return null;
    const i = Math.max(r.integ_wt.lastIndexOf("/"), r.integ_wt.lastIndexOf("\\"));
    return `${r.integ_wt.slice(0, i + 1)}${r.name}-${id}`;
  };
  // the reason of a failed / needs-review card in the UI language (the daemon's English text when it sent no code)
  const why = (f: [string, string]) => {
    const c = run().reason_codes?.[f[0]];
    return coded(c?.code, c?.params, f[1]);
  };
  const reason = (id: string) => {
    const r = run().needs_review?.find((x) => x[0] === id);
    return r ? why(r) : P.kanban.waitingReview;
  };
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
    // eslint-disable-next-line solid/reactivity -- act() is only called from click handlers
    act(async () => {
      const acc = c.meta.acceptance;
      if (acc && !(await ask(P.kanban.retryAsk(c.meta.title || c.meta.id, acc), { title: P.kanban.retryTitle, kind: "warning" }))) return;
      await orchRetry(props.project, c.meta.id, !!acc);
    });
  const openPr = (c: KCard) =>
    // eslint-disable-next-line solid/reactivity -- act() is only called from click handlers
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
            <Show when={(run().failed?.length ?? 0) > 0}>{P.kanban.failedBanner(run().failed!.map((f) => `${f[0]} (${why(f)})`).join(", "))}</Show>
            <Show when={(run().needs_review?.length ?? 0) > 0}>{P.kanban.reviewBanner(run().needs_review!.map((f) => `${f[0]} (${why(f)})`).join(", "))}</Show>
            <Show when={(run().blocked?.length ?? 0) > 0}>{P.kanban.blockedBanner(run().blocked!.join(", "))}</Show>
          </Show>
          <Show when={run().state === "error"}>{P.kanban.errorBanner(coded(run().error_code, run().error_params, run().error ?? ""))}</Show>
        </div>
      </Show>
      <Show when={board().broken.length > 0}>
        <div class="kerr">
          {P.kanban.broken}{" "}
          <For each={board().broken}>{(b) => <span title={coded(b.error_code, b.error_params, b.error)}>{b.file} </span>}</For>
        </div>
      </Show>
      <div class="sr-only" id="kanban-grab-hint">{P.kanban.grabHint}</div>
      <div class="sr-only" role="status" aria-live="assertive" aria-atomic="true">{live()}</div>
      <Show when={drag()?.mode === "ptr"}>
        <div
          class="kghost"
          ref={(el) => {
            ghostEl = el;
            if (down) el.style.setProperty("--gw", `${down.w}px`);
            place();
          }}
        >
          <div class="kcard-title">{cardName(drag()!.id)}</div>
        </div>
      </Show>
      <div class="kcols" classList={{ kdragging: drag()?.mode === "ptr" }} ref={colsEl}>
        <For each={COLS}>
          {(col) => (
            <div class="kcol" classList={{ over: drag()?.col === col.id }}>
              <div class="kcol-head">
                <span>{col.label}</span>
                <span class="kcount">{colCards(col.id).length}</span>
                <Button variant="icon" size="icon" title={P.kanban.newCard} aria-label={P.kanban.newCard} onClick={() => edit(undefined, col.id)}>
                  <Icon name="plus" />
                </Button>
              </div>
              <div class="kcards" classList={{ "drop-end": dropEnd(col.id) }}>
                <For each={colCards(col.id)}>
                  {(c) => (
                    <div
                      class={`kcard st-${col.id}`}
                      classList={{ dragging: drag()?.id === c.meta.id, "drop-before": dropBefore(col.id, c.meta.id), grabbed: drag()?.mode === "kbd" && drag()?.id === c.meta.id }}
                      data-id={c.meta.id}
                      role="button"
                      tabindex="0"
                      aria-describedby="kanban-grab-hint"
                      onPointerDown={(e) => onDown(e, c)}
                      onClick={() => !suppressClick && edit(c)}
                      onKeyDown={(e) => onCardKey(e, c)}
                      onBlur={() => drag()?.mode === "kbd" && drag()?.id === c.meta.id && cancelDrag()}
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
