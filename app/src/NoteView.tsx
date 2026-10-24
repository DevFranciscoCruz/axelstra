import { For, Match, Show, Switch, createResource, createSignal } from "solid-js";
import { Button } from "./ui";
import { Modal } from "./Modal";
import { ask } from "./Confirm";
import { notesExport, notesRead, notesReadFile } from "./api";
import { parseNote } from "./noteMd";
import { P } from "./strings.panels";

/** the task file that is open: an agent's session, or a file by project and name (an agent that already ended); null = closed */
type Target = { session: number; name: string } | { project: string; name: string };
const [target, setTarget] = createSignal<Target | null>(null);
export const openNote = (session: number, name: string) => setTarget({ session, name });
export const openNoteFile = (project: string, name: string) => setTarget({ project, name });

/** Read-only view of an agent's task file. The file lives outside the repository, so the editor cannot open it;
 *  "Exportar para o repositório" copies it into the worktree's .axelstra folder. */
export function NoteView(props: { files?: boolean }) {
  // two hosts: the sidebar shows the notes of live sessions, the empty state of a worktree those of ended agents (`files`)
  const mine = () => {
    const t = target();
    return t && "session" in t !== !!props.files ? t : null;
  };
  return <Show when={mine()}>{(t) => <Body target={t()} />}</Show>;
}

function Body(props: { target: Target }) {
  const name = props.target.name;
  // only a live session can export: the daemon finds the worktree through the session
  const session = "session" in props.target ? props.target.session : null;
  const [note] = createResource(
    () => props.target,
    (t) => ("session" in t ? notesRead(t.session) : notesReadFile(t.project, t.name)),
  );
  const [status, setStatus] = createSignal<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = createSignal(false);
  const close = () => setTarget(null);

  const exportIt = async (overwrite: boolean) => {
    setBusy(true);
    try {
      const r = await notesExport(session!, overwrite);
      setStatus({ ok: true, text: P.note.exported(r.path) });
    } catch (e) {
      const msg = String(e);
      if (!overwrite && /already exists/i.test(msg)) {
        if (await ask(P.note.replaceAsk, { title: P.note.exportTitle, kind: "warning" })) return exportIt(true);
        setStatus({ ok: false, text: P.note.kept });
      } else setStatus({ ok: false, text: msg });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal label={P.note.label(name)} onClose={close} class="modal note-view">
      <div class="m-title font-semibold">{P.note.heading(name)}</div>
      <Switch>
        <Match when={note.error}>
          <div class="note-err" role="alert">{String(note.error)}</div>
        </Match>
        <Match when={note()}>
          {(n) => (
            <>
              <div class="note-path" title={n().path}>{n().path}</div>
              <div class="note-body" tabindex="0" role="document" aria-label={P.note.textLabel(name)}>
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
          <div class="note-p">{P.note.loading}</div>
        </Match>
      </Switch>
      <Show when={status()}>
        {(s) => <div class="note-status" classList={{ bad: !s().ok }} role={s().ok ? "status" : "alert"}>{s().text}</div>}
      </Show>
      <div class="m-actions">
        <Button size="sm" onClick={close}>{P.note.close}</Button>
        <Button variant="default" size="sm" hidden={session === null} disabled={busy() || !note()} onClick={() => void exportIt(false)}>
          {P.note.exportBtn}
        </Button>
      </div>
    </Modal>
  );
}
