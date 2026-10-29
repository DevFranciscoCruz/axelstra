import { For, Show, createResource, createSignal, onCleanup } from "solid-js";
import { Icon } from "./Icon";
import { createLive } from "./live";
import { Automation, agentOptions, agentWarning, agentsList, deleteAutomation, listAutomations, runAutomation, saveAutomation } from "./api";
import { Modal } from "./Modal";
import { ask, askDiscard } from "./Confirm";
import { projects, refreshProjects, refreshSessions, navStamp, selectIfStill, setError, setShowAutomations, toast } from "./store";
import { S } from "./strings";
import { P } from "./strings.panels";
import { Button, Field, Input, Select, Textarea } from "./ui";

const days = () => P.automations.days;

const describe = (a: Automation) => {
  const s = a.schedule;
  if (s.kind === "interval") return P.automations.everyMin(s.minutes);
  if (s.kind === "daily") return P.automations.daily(s.time);
  return P.automations.weekly(days()[s.day], s.time);
};

const blank = (project: string): Automation => ({
  id: "",
  name: "",
  project,
  agent: "claude",
  prompt: "",
  schedule: { kind: "daily", time: "09:00" },
  enabled: true,
  last_run: 0,
});

/** Scheduled agent runs: each one gets its own worktree and an agent started with the prompt. */
export function Automations() {
  const [list, { refetch }] = createResource(listAutomations);
  const [agents] = createResource(() => agentsList().catch((e) => (console.debug("agentsList", e), undefined)));
  // the daemon says `automations.changed` when one was saved, deleted or run (by the scheduler too): reload then, never on a timer
  const live = createLive(() => refetch(), ["automations.changed"]);
  onCleanup(live.stop);
  const [draft, setDraft] = createSignal<Automation | null>(null);
  // the draft as it was when the editor opened: leaving with anything different asks first
  const [base, setBase] = createSignal("");
  const edit = (a: Automation) => {
    setBase(JSON.stringify(a));
    setDraft(a);
  };
  const dirty = () => draft() !== null && JSON.stringify(draft()) !== base();
  const leaveDraft = async () => {
    if (dirty() && !(await askDiscard(P.automations.unsavedDraft))) return;
    setDraft(null);
  };
  const [msg, setMsg] = createSignal("");

  const projectName = (id: string) => projects().find((p) => p.id === id)?.name ?? id;

  const guard = async (f: () => Promise<unknown>) => {
    try {
      setMsg("");
      await f();
    } catch (e) {
      setMsg(String(e));
    }
  };

  const save = () =>
    // eslint-disable-next-line solid/reactivity -- guard() runs this from an event handler
    guard(async () => {
      await saveAutomation(draft()!);
      setDraft(null);
      await refetch();
    });

  const runNow = async (a: Automation) => {
    const ok = await ask(P.automations.runAsk(a.name, describe(a)), { title: P.automations.runTitle, kind: "info", okLabel: P.automations.run, cancelLabel: P.automations.cancel });
    if (ok) await doRun(a);
  };
  const doRun = (a: Automation) =>
    guard(async () => {
      const stamp = navStamp();
      const r = await runAutomation(a.id);
      await refreshProjects();
      await refreshSessions();
      setShowAutomations(false);
      selectIfStill(stamp, r.worktree);
    });

  const toggle = (a: Automation) =>
    guard(async () => {
      await saveAutomation({ ...a, enabled: !a.enabled });
      await refetch();
    });

  const remove = async (a: Automation) => {
    try {
      await deleteAutomation(a.id);
      // the deleted object is kept: undo saves it again under the same id
      toast({
        text: S.automationDeleted(a.name),
        kind: "info",
        action: { label: P.automations.undo, run: () => void saveAutomation(a).then(() => refetch(), (e) => setError(S.deleteFailed(a.name, e))) },
      });
      await refetch();
    } catch (e) {
      setError(S.deleteFailed(a.name, e));
    }
  };

  return (
    <Modal label={P.automations.title} class="modal auto" dirty={dirty()} onClose={() => (draft() ? void leaveDraft() : setShowAutomations(false))}>
        <Show
          when={draft()}
          fallback={
            <>
              <div class="auto-head">
                <strong>{P.automations.title}</strong>
                <span class="grow" />
                <Button variant="default" disabled={projects().length === 0} title={projects().length === 0 ? P.automations.addFirst : undefined} onClick={() => edit(blank(projects()[0].id))}>
                  {P.automations.new}
                </Button>
                <Button onClick={() => setShowAutomations(false)}>{P.automations.close}</Button>
              </div>
              <div class="auto-note">
                {P.automations.note}
              </div>
              <div class="auto-list">
                <For each={list() ?? []} fallback={
                  <div class="placeholder empty">
                    <div>{P.automations.none}</div>
                  </div>
                }>
                  {(a) => (
                    <div class="auto-item" classList={{ off: !a.enabled }}>
                      <div class="auto-main">
                        <div class="auto-name">{a.name}</div>
                        <div class="auto-sub">
                          {projectName(a.project)} · {a.agent} · {describe(a)}
                        </div>
                      </div>
                      <Button size="icon" variant="ghost" title={P.automations.runNowTip} aria-label={P.automations.runNowLabel(a.name)} onClick={() => void runNow(a)}>
                        <Icon name="bolt" />
                      </Button>
                      <Button size="icon" variant="ghost" title={a.enabled ? P.automations.disableTip : P.automations.enableTip} aria-label={P.automations.toggleLabel(a.enabled, a.name)} onClick={() => void toggle(a)}>
                        <Icon name={a.enabled ? "pause" : "play"} />
                      </Button>
                      <Button size="icon" variant="ghost" title={P.automations.edit} aria-label={P.automations.edit} onClick={() => edit({ ...a })}>
                        <Icon name="pencil" />
                      </Button>
                      <Button size="icon" variant="ghost" class="text-destructive" title={P.automations.delete} aria-label={P.automations.delete} onClick={() => void remove(a)}>
                        <Icon name="x" />
                      </Button>
                    </div>
                  )}
                </For>
              </div>
              <span class="cmt-msg" role="status" aria-live="polite">{msg()}</span>
            </>
          }
        >
          {(d) => (
            <>
              <div class="auto-head">
                <strong>{d().id ? P.automations.editTitle : P.automations.newTitle}</strong>
              </div>
              <Input
                class="m-title"
                placeholder={P.automations.namePlaceholder}
                value={d().name}
                onInput={(e) => setDraft({ ...d(), name: e.currentTarget.value })}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
              <Textarea
                class="m-body"
                placeholder={P.automations.promptPlaceholder}
                value={d().prompt}
                onInput={(e) => setDraft({ ...d(), prompt: e.currentTarget.value })}
              />
              <div class="m-row">
                <Field label={P.automations.project}>
                  <Select
                    value={d().project}
                    options={projects().map((p) => ({ value: p.id, label: p.name }))}
                    onChange={(v) => setDraft({ ...d(), project: v })}
                  />
                </Field>
                <Field label={P.automations.agent}>
                  <Select
                    value={d().agent}
                    options={agentOptions(agents() ?? [], d().agent)}
                    onChange={(v) => setDraft({ ...d(), agent: v })}
                  />
                  <Show when={agentWarning(agents() ?? [], d().agent)}>{(w) => <span class="auto-note">{w()}</span>}</Show>
                </Field>
              </div>
              <div class="m-row">
                <Field label={P.automations.repeat}>
                  <Select
                    value={d().schedule.kind}
                    options={[
                      { value: "interval", label: P.automations.kinds.interval },
                      { value: "daily", label: P.automations.kinds.daily },
                      { value: "weekly", label: P.automations.kinds.weekly },
                    ]}
                    onChange={(k) =>
                      setDraft({
                        ...d(),
                        schedule: k === "interval" ? { kind: "interval", minutes: 60 } : k === "daily" ? { kind: "daily", time: "09:00" } : { kind: "weekly", day: 0, time: "09:00" },
                      })
                    }
                  />
                </Field>
                {(() => {
                  const s = () => d().schedule;
                  // each field narrows the schedule by its kind: no casts
                  const interval = () => { const x = s(); return x.kind === "interval" ? x : undefined; };
                  const weekly = () => { const x = s(); return x.kind === "weekly" ? x : undefined; };
                  const timed = () => { const x = s(); return x.kind === "interval" ? undefined : x; };
                  return (
                    <>
                      <Show when={interval()}>
                        {(x) => (
                          <Field label={P.automations.minutes}>
                            <Input
                              type="number"
                              min="5"
                              value={x().minutes}
                              onInput={(e) => setDraft({ ...d(), schedule: { kind: "interval", minutes: Math.max(5, parseInt(e.currentTarget.value) || 5) } })}
                            />
                          </Field>
                        )}
                      </Show>
                      <Show when={weekly()}>
                        {(x) => (
                          <Field label={P.automations.day}>
                            <Select
                              value={String(x().day)}
                              options={days().map((n, idx) => ({ value: String(idx), label: n }))}
                              onChange={(v) => setDraft({ ...d(), schedule: { ...x(), day: parseInt(v) } })}
                            />
                          </Field>
                        )}
                      </Show>
                      <Show when={timed()}>
                        {(x) => (
                          <Field label={P.automations.time}>
                            <Input type="time" value={x().time} onInput={(e) => setDraft({ ...d(), schedule: { ...x(), time: e.currentTarget.value } })} />
                          </Field>
                        )}
                      </Show>
                    </>
                  );
                })()}
              </div>
              <div class="m-actions">
                <span class="cmt-msg grow" role="status" aria-live="polite">{msg()}</span>
                <Button onClick={() => void leaveDraft()}>{P.automations.cancel}</Button>
                <Button variant="default" disabled={!d().name.trim() || !d().prompt.trim()} onClick={() => void save()}>
                  {P.automations.save}
                </Button>
              </div>
            </>
          )}
        </Show>
    </Modal>
  );
}
