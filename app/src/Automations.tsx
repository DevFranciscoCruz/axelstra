import { For, Show, createResource, createSignal } from "solid-js";
import { Icon } from "./Icon";
import { Automation, agentOptions, agentWarning, agentsList, deleteAutomation, listAutomations, runAutomation, saveAutomation } from "./api";
import { Modal } from "./Modal";
import { ask, askDiscard } from "./Confirm";
import { projects, refreshProjects, refreshSessions, selectWorktree, setError, setShowAutomations, showNotice, toast } from "./store";
import { S } from "./strings";
import { Button, Field, Input, Select, Textarea } from "./ui";

const DAYS = ["Segunda", "Terça", "Quarta", "Quinta", "Sexta", "Sábado", "Domingo"];

const describe = (a: Automation) => {
  const s = a.schedule;
  if (s.kind === "interval") return `a cada ${s.minutes} min`;
  if (s.kind === "daily") return `todos os dias às ${s.time}`;
  return `${DAYS[s.day]} às ${s.time}`;
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
  const [agents] = createResource(agentsList);
  const [draft, setDraft] = createSignal<Automation | null>(null);
  // the draft as it was when the editor opened: leaving with anything different asks first
  const [base, setBase] = createSignal("");
  const edit = (a: Automation) => {
    setBase(JSON.stringify(a));
    setDraft(a);
  };
  const dirty = () => draft() !== null && JSON.stringify(draft()) !== base();
  const leaveDraft = async () => {
    if (dirty() && !(await askDiscard("O rascunho tem alterações por guardar."))) return;
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
    guard(async () => {
      await saveAutomation(draft()!);
      setDraft(null);
      await refetch();
    });

  const runNow = async (a: Automation) => {
    const ok = await ask(`Executar "${a.name}" agora?

Abre já um worktree novo e põe o agente a trabalhar no pedido, fora do horário (${describe(a)}).`, { title: "Executar agora", kind: "info", okLabel: "Executar", cancelLabel: "Cancelar" });
    if (ok) await doRun(a);
  };
  const doRun = (a: Automation) =>
    guard(async () => {
      const r = await runAutomation(a.id);
      await refreshProjects();
      await refreshSessions();
      setShowAutomations(false);
      selectWorktree(r.worktree);
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
        action: { label: "Desfazer", run: () => void saveAutomation(a).then(() => refetch(), (e) => setError(S.deleteFailed(a.name, e))) },
      });
      await refetch();
    } catch (e) {
      setError(S.deleteFailed(a.name, e));
    }
  };

  return (
    <Modal label="Automatizações" class="modal auto" dirty={dirty()} onClose={() => (draft() ? void leaveDraft() : setShowAutomations(false))}>
        <Show
          when={draft()}
          fallback={
            <>
              <div class="auto-head">
                <strong>Automatizações</strong>
                <span class="grow" />
                <Button variant="default" disabled={projects().length === 0} title={projects().length === 0 ? "Adiciona um projeto primeiro" : undefined} onClick={() => edit(blank(projects()[0].id))}>
                  + Nova
                </Button>
                <Button onClick={() => setShowAutomations(false)}>Fechar</Button>
              </div>
              <div class="auto-note">
                Cada execução abre um worktree novo com o agente já a trabalhar no pedido. Só corre com o Axelstra aberto
                (o daemon); execuções perdidas são ignoradas.
              </div>
              <div class="auto-list">
                <For each={list() ?? []} fallback={
                  <div class="placeholder empty">
                    <div>Nenhuma automatização</div>
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
                      <Button size="icon" variant="ghost" title="Executar agora (fora do horário)" aria-label={`Executar agora: ${a.name}`} onClick={() => void runNow(a)}>
                        <Icon name="bolt" />
                      </Button>
                      <Button size="icon" variant="ghost" title={a.enabled ? "Desativar o agendamento" : "Ativar o agendamento"} aria-label={`${a.enabled ? "Desativar" : "Ativar"} o agendamento: ${a.name}`} onClick={() => void toggle(a)}>
                        <Icon name={a.enabled ? "pause" : "play"} />
                      </Button>
                      <Button size="icon" variant="ghost" title="Editar" aria-label="Editar" onClick={() => edit({ ...a })}>
                        <Icon name="pencil" />
                      </Button>
                      <Button size="icon" variant="ghost" class="text-destructive" title="Apagar automatização" aria-label="Apagar automatização" onClick={() => void remove(a)}>
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
                <strong>{d().id ? "Editar automatização" : "Nova automatização"}</strong>
              </div>
              <Input
                class="m-title"
                placeholder="Nome (ex.: Revisão da manhã)"
                value={d().name}
                onInput={(e) => setDraft({ ...d(), name: e.currentTarget.value })}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
              <Textarea
                class="m-body"
                placeholder="O que o agente deve fazer em cada execução…"
                value={d().prompt}
                onInput={(e) => setDraft({ ...d(), prompt: e.currentTarget.value })}
              />
              <div class="m-row">
                <Field label="Projeto">
                  <Select
                    value={d().project}
                    options={projects().map((p) => ({ value: p.id, label: p.name }))}
                    onChange={(v) => setDraft({ ...d(), project: v })}
                  />
                </Field>
                <Field label="Agente">
                  <Select
                    value={d().agent}
                    iconOnly
                  options={agentOptions(agents() ?? [], d().agent)}
                    onChange={(v) => setDraft({ ...d(), agent: v })}
                  />
                  <Show when={agentWarning(agents() ?? [], d().agent)}>{(w) => <span class="auto-note">{w()}</span>}</Show>
                </Field>
              </div>
              <div class="m-row">
                <Field label="Repetição">
                  <Select
                    value={d().schedule.kind}
                    options={[
                      { value: "interval", label: "A cada N minutos" },
                      { value: "daily", label: "Todos os dias" },
                      { value: "weekly", label: "Todas as semanas" },
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
                  return (
                    <>
                      <Show when={s().kind === "interval"}>
                        <Field label="Minutos">
                          <Input
                            type="number"
                            min="5"
                            value={(s() as { minutes: number }).minutes}
                            onInput={(e) => setDraft({ ...d(), schedule: { kind: "interval", minutes: Math.max(5, parseInt(e.currentTarget.value) || 5) } })}
                          />
                        </Field>
                      </Show>
                      <Show when={s().kind === "weekly"}>
                        <Field label="Dia">
                          <Select
                            value={String((s() as { day: number }).day)}
                            options={DAYS.map((n, idx) => ({ value: String(idx), label: n }))}
                            onChange={(v) => setDraft({ ...d(), schedule: { ...(s() as { kind: "weekly"; day: number; time: string }), day: parseInt(v) } })}
                          />
                        </Field>
                      </Show>
                      <Show when={s().kind !== "interval"}>
                        <Field label="Hora">
                          <Input
                            type="time"
                            value={(s() as { time: string }).time}
                            onInput={(e) => setDraft({ ...d(), schedule: { ...(s() as { kind: "daily"; time: string }), time: e.currentTarget.value } })}
                          />
                        </Field>
                      </Show>
                    </>
                  );
                })()}
              </div>
              <div class="m-actions">
                <span class="cmt-msg grow" role="status" aria-live="polite">{msg()}</span>
                <Button onClick={() => void leaveDraft()}>Cancelar</Button>
                <Button variant="default" disabled={!d().name.trim() || !d().prompt.trim()} onClick={() => void save()}>
                  Guardar
                </Button>
              </div>
            </>
          )}
        </Show>
    </Modal>
  );
}
