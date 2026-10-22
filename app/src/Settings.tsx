import { For, Show, createResource } from "solid-js";
import { Terse, getSettings, setSettings } from "./api";
import { Modal } from "./Modal";
import { setShowSettings, showNotice } from "./store";

const TERSE: { value: Terse; label: string; hint: string }[] = [
  { value: "off", label: "Normais", hint: "Os agentes respondem como sempre." },
  { value: "short", label: "Curtas", hint: "Resposta primeiro, sem enrolação nem resumo no fim." },
  { value: "tiny", label: "Mínimas", hint: "Frases soltas, no máximo três frases, só o essencial." },
];

/** App-wide preferences. Each change is saved at once; agents already open keep their current behaviour. */
export function Settings() {
  const [s, { mutate }] = createResource(getSettings);
  const save = async (patch: { terse?: Terse; pm?: string | null; auto_install?: boolean }) => {
    try {
      await setSettings(patch);
      mutate((cur) => (cur ? { ...cur, ...patch } : cur));
    } catch (e) {
      showNotice(String(e));
    }
  };
  return (
    <Modal label="Definições" class="modal settings" onClose={() => setShowSettings(false)}>
      <div class="auto-head">
        <strong>Definições</strong>
        <span class="grow" />
        <button onClick={() => setShowSettings(false)}>Fechar</button>
      </div>
      <Show when={s()} fallback={<div class="auto-note">A carregar…</div>}>
        {(cfg) => (
          <>
            <section class="set-sec">
              <h4>Respostas dos agentes</h4>
              <p class="auto-note">Poupa tokens e encurta os relatórios dos workers. Vale para agentes novos.</p>
              <div class="set-choices" role="radiogroup" aria-label="Respostas dos agentes">
                <For each={TERSE}>
                  {(o) => (
                    <button role="radio" aria-checked={cfg().terse === o.value} classList={{ on: cfg().terse === o.value }} title={o.hint} onClick={() => void save({ terse: o.value })}>
                      {o.label}
                    </button>
                  )}
                </For>
              </div>
              <div class="auto-note">{TERSE.find((o) => o.value === cfg().terse)?.hint}</div>
            </section>

            <section class="set-sec">
              <h4>Gerenciador de pacotes</h4>
              <p class="auto-note">Só aparecem os que estão instalados nesta máquina. O lockfile do projeto tem prioridade sobre esta escolha.</p>
              <Show when={cfg().installed.length > 0} fallback={<div class="auto-note">Nenhum encontrado no PATH (bun, pnpm, yarn, npm).</div>}>
                <div class="set-choices" role="radiogroup" aria-label="Gerenciador de pacotes">
                  <button role="radio" aria-checked={cfg().pm === null} classList={{ on: cfg().pm === null }} onClick={() => void save({ pm: null })}>
                    Automático
                  </button>
                  <For each={cfg().installed}>
                    {(m) => (
                      <button role="radio" aria-checked={cfg().pm === m} classList={{ on: cfg().pm === m }} onClick={() => void save({ pm: m })}>
                        {m}
                      </button>
                    )}
                  </For>
                </div>
              </Show>
              <label class="set-check">
                <input type="checkbox" checked={cfg().auto_install} onChange={(e) => void save({ auto_install: e.currentTarget.checked })} />
                Instalar dependências ao criar um worktree (projetos com package.json e sem comando de setup)
              </label>
            </section>
          </>
        )}
      </Show>
    </Modal>
  );
}
