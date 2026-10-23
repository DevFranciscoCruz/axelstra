import { For, Show, createResource, createSignal } from "solid-js";
import { projectEnvPreview, setProjectSetup } from "./api";
import { Modal } from "./Modal";
import { projects, refreshProjects, setError } from "./store";

const LIST = { margin: "0", "padding-left": "18px", "font-family": "monospace", "font-size": "12px", "line-height": "1.5" };

/** Per-project settings: the setup command of new worktrees, and a preview of the files `.worktreeinclude` copies. */
export function ProjectSettings(props: { projectId: string; onClose: () => void }) {
  const project = () => projects().find((p) => p.id === props.projectId);
  const [setup, setSetup] = createSignal(project()?.setup ?? "");
  const [saving, setSaving] = createSignal(false);
  const [preview, { refetch }] = createResource(() => props.projectId, projectEnvPreview);
  const dirty = () => setup().trim() !== (project()?.setup ?? "").trim();

  const save = async () => {
    setSaving(true);
    try {
      await setProjectSetup(props.projectId, setup().trim());
      await refreshProjects();
      props.onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal label={`Definições de ${project()?.name ?? "projeto"}`} class="modal" dirty={dirty()} onClose={props.onClose}>
      <div class="auto-head">
        <strong>{project()?.name}</strong>
        <span class="grow" />
        <button onClick={props.onClose}>Fechar</button>
      </div>

      <label for="ps-setup">Comando de setup</label>
      <input
        id="ps-setup"
        placeholder="ex.: bun install (vazio = nenhum)"
        value={setup()}
        onInput={(e) => setSetup(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !saving()) void save();
        }}
      />
      <div class="auto-note">
        Corre num terminal visível de cada worktree novo. Fica guardado só nesta app, nunca no repositório: um repositório clonado não consegue
        mandar o Axelstra correr comandos.
      </div>

      <label>Ficheiros copiados para worktrees novos</label>
      <div class="auto-note">
        Lista no ficheiro <code>.worktreeinclude</code>, na raiz do projeto, os padrões (sintaxe do .gitignore) dos ficheiros ignorados pelo git
        a copiar do checkout principal, por exemplo <code>.env</code> ou <code>.env.local</code>. Máximo de 50 ficheiros de 5 MB cada.
      </div>
      <Show when={preview.state === "ready"} fallback={<div class="auto-note">{preview.error ? "Não foi possível ler a lista." : "A ler…"}</div>}>
        <Show
          when={preview()!.has_file}
          fallback={<div class="auto-note">Este projeto não tem <code>.worktreeinclude</code>: nada será copiado.</div>}
        >
          <Show when={preview()!.files.length} fallback={<div class="auto-note">Nenhum ficheiro ignorado corresponde aos padrões.</div>}>
            <ul style={LIST} aria-label="Ficheiros que serão copiados">
              <For each={preview()!.files}>{(f) => <li>{f}</li>}</For>
            </ul>
          </Show>
          <Show when={preview()!.skipped.length}>
            <div class="auto-note">Ficam de fora:</div>
            <ul style={LIST} aria-label="Ficheiros que ficam de fora">
              <For each={preview()!.skipped}>
                {(s) => (
                  <li>
                    {s.path} <span class="auto-note">— {s.reason}</span>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </Show>
      </Show>

      <div class="m-actions">
        <button onClick={() => void refetch()}>Atualizar lista</button>
        <span class="grow" />
        <button class="primary" disabled={saving() || !dirty()} onClick={() => void save()}>
          Guardar
        </button>
      </div>
    </Modal>
  );
}
