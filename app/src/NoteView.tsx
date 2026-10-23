import { For, Match, Show, Switch, createResource, createSignal } from "solid-js";
import { Modal } from "./Modal";
import { ask } from "./Confirm";
import { notesExport, notesRead } from "./api";
import { parseNote } from "./noteMd";

/** the agent whose task file is open (session id and the name shown); null = closed */
const [target, setTarget] = createSignal<{ session: number; name: string } | null>(null);
export const openNote = (session: number, name: string) => setTarget({ session, name });

/** Read-only view of an agent's task file. The file lives outside the repository, so the editor cannot open it;
 *  "Exportar para o repositório" copies it into the worktree's .axelstra folder. */
export function NoteView() {
  return <Show when={target()}>{(t) => <Body session={t().session} name={t().name} />}</Show>;
}

function Body(props: { session: number; name: string }) {
  const [note] = createResource(() => props.session, notesRead);
  const [status, setStatus] = createSignal<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = createSignal(false);
  const close = () => setTarget(null);

  const exportIt = async (overwrite: boolean) => {
    setBusy(true);
    try {
      const r = await notesExport(props.session, overwrite);
      setStatus({ ok: true, text: `Exportado para ${r.path}` });
    } catch (e) {
      const msg = String(e);
      if (!overwrite && /already exists/i.test(msg)) {
        if (await ask("Já existe um relatório com este nome em .axelstra. Substituir?", { title: "Exportar", kind: "warning" })) return exportIt(true);
        setStatus({ ok: false, text: "Ficheiro existente mantido." });
      } else setStatus({ ok: false, text: msg });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal label={`Relatório de ${props.name}`} onClose={close} class="modal note-view">
      <div class="m-title" style={{ "font-weight": "600" }}>Relatório · {props.name}</div>
      <Switch>
        <Match when={note.error}>
          <div class="note-err" role="alert">{String(note.error)}</div>
        </Match>
        <Match when={note()}>
          {(n) => (
            <>
              <div class="note-path" title={n().path}>{n().path}</div>
              <div class="note-body" tabindex="0" role="document" aria-label={`Texto do relatório de ${props.name}`}>
                <For each={parseNote(n().text)}>
                  {(b) =>
                    b.kind === "h" ? (
                      <div class={`note-h note-h${Math.min(b.level, 3)}`} role="heading" aria-level={b.level}>{b.text}</div>
                    ) : b.kind === "li" ? (
                      <div class="note-li">{b.text}</div>
                    ) : b.kind === "code" ? (
                      <pre class="note-code">{b.text}</pre>
                    ) : (
                      <p class="note-p">{b.text}</p>
                    )
                  }
                </For>
              </div>
            </>
          )}
        </Match>
        <Match when={true}>
          <div class="note-p">A carregar…</div>
        </Match>
      </Switch>
      <Show when={status()}>
        {(s) => <div class="note-status" classList={{ bad: !s().ok }} role={s().ok ? "status" : "alert"}>{s().text}</div>}
      </Show>
      <div class="m-actions" style={{ "justify-content": "flex-end" }}>
        <button onClick={close}>Fechar</button>
        <button class="primary" disabled={busy() || !note()} onClick={() => void exportIt(false)}>
          Exportar para o repositório
        </button>
      </div>
    </Modal>
  );
}
