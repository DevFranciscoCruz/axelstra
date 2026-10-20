import { For, Show, createResource, createSignal } from "solid-js";
import { Automation, deleteAutomation, listAutomations, runAutomation, saveAutomation } from "./api";
import { Select } from "./Select";
import { Modal } from "./Modal";
import { ask } from "./Confirm";
import { projects, refreshProjects, refreshSessions, selectWorktree, setError, setShowAutomations } from "./store";

const DAYS = ["Segunda", "Terça", "Quarta", "Quinta", "Sexta", "Sábado", "Domingo"];
const AGENT_CLI = [
  ["claude", "claude"],
  ["codex", "chatgpt (codex)"],
] as const;

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
  const [draft, setDraft] = createSignal<Automation | null>(null);
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

  const runNow = (a: Automation) =>
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
    const ok = await ask(`Apagar a automatização "${a.name}"?`, { title: "Apagar", kind: "warning", okLabel: "Apagar", cancelLabel: "Cancelar" });
    if (!ok) return;
    await guard(async () => {
      await deleteAutomation(a.id);
      await refetch();
    });
  };

  return (
    <Modal label="Automatizações" class="modal auto" dirty={draft() !== null} onClose={() => (draft() ? setDraft(null) : setShowAutomations(false))}>
        <Show
          when={draft()}
          fallback={
            <>
              <div class="auto-head">
                <strong>Automatizações</strong>
                <span class="grow" />
                <button class="primary" disabled={projects().length === 0} title={projects().length === 0 ? "Adicione um projeto primeiro" : undefined} onClick={() => setDraft(blank(projects()[0].id))}>
                  + Nova
                </button>
                <button onClick={() => setShowAutomations(false)}>Fechar</button>
              </div>
              <div class="auto-note">
                Cada execução abre um worktree novo com o agente já a trabalhar no pedido. Só corre com o Axelstra aberto
                (o daemon); execuções perdidas são ignoradas.
              </div>
              <div class="auto-list">
                <For each={list() ?? []} fallback={<div class="placeholder">Nenhuma automatização</div>}>
                  {(a) => (
                    <div class="auto-item" classList={{ off: !a.enabled }}>
                      <div class="auto-main">
                        <div class="auto-name">{a.name}</div>
                        <div class="auto-sub">
                          {projectName(a.project)} · {a.agent} · {describe(a)}
                        </div>
                      </div>
                      <button class="icon" title="Executar agora" onClick={() => void runNow(a)}>
                        ▶
                      </button>
                      <button class="icon" title={a.enabled ? "Desativar" : "Ativar"} onClick={() => void toggle(a)}>
                        {a.enabled ? "⏸" : "⏵"}
                      </button>
                      <button class="icon" title="Editar" onClick={() => setDraft({ ...a })}>
                        ✎
                      </button>
                      <button class="icon danger" title="Apagar automatização" onClick={() => void remove(a)}>
                        ×
                      </button>
                    </div>
                  )}
                </For>
              </div>
              <span class="cmt-msg">{msg()}</span>
            </>
          }
        >
          {(d) => (
            <>
              <div class="auto-head">
                <strong>{d().id ? "Editar automatização" : "Nova automatização"}</strong>
              </div>
              <input
                class="m-title"
                placeholder="Nome (ex.: Revisão da manhã)"
                value={d().name}
                onInput={(e) => setDraft({ ...d(), name: e.currentTarget.value })}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
              <textarea
                class="m-body"
                placeholder="O que o agente deve fazer em cada execução…"
                value={d().prompt}
                onInput={(e) => setDraft({ ...d(), prompt: e.currentTarget.value })}
              />
              <div class="m-row">
                <label>
                  Projeto
                  <Select
                    value={d().project}
                    options={projects().map((p) => ({ value: p.id, label: p.name }))}
                    onChange={(v) => setDraft({ ...d(), project: v })}
                  />
                </label>
                <label>
                  Agente
                  <Select
                    value={d().agent}
                    options={AGENT_CLI.map(([v, l]) => ({ value: v, label: l }))}
                    onChange={(v) => setDraft({ ...d(), agent: v })}
                  />
                </label>
              </div>
              <div class="m-row">
                <label>
                  Repetição
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
                </label>
                {(() => {
                  const s = () => d().schedule;
                  return (
                    <>
                      <Show when={s().kind === "interval"}>
                        <label>
                          Minutos
                          <input
                            type="number"
                            min="5"
                            value={(s() as { minutes: number }).minutes}
                            onInput={(e) => setDraft({ ...d(), schedule: { kind: "interval", minutes: Math.max(5, parseInt(e.currentTarget.value) || 5) } })}
                          />
                        </label>
                      </Show>
                      <Show when={s().kind === "weekly"}>
                        <label>
                          Dia
                          <Select
                            value={String((s() as { day: number }).day)}
                            options={DAYS.map((n, idx) => ({ value: String(idx), label: n }))}
                            onChange={(v) => setDraft({ ...d(), schedule: { ...(s() as { kind: "weekly"; day: number; time: string }), day: parseInt(v) } })}
                          />
                        </label>
                      </Show>
                      <Show when={s().kind !== "interval"}>
                        <label>
                          Hora
                          <input
                            type="time"
                            value={(s() as { time: string }).time}
                            onInput={(e) => setDraft({ ...d(), schedule: { ...(s() as { kind: "daily"; time: string }), time: e.currentTarget.value } })}
                          />
                        </label>
                      </Show>
                    </>
                  );
                })()}
              </div>
              <div class="m-actions">
                <span class="cmt-msg grow">{msg()}</span>
                <button onClick={() => setDraft(null)}>Cancelar</button>
                <button class="primary" disabled={!d().name.trim() || !d().prompt.trim()} onClick={() => void save()}>
                  Guardar
                </button>
              </div>
            </>
          )}
        </Show>
    </Modal>
  );
}
