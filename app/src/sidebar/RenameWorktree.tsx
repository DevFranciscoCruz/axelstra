import { Show } from "solid-js";
import { Modal } from "../Modal";
import { Button, Field, Input } from "../ui";

/** The "Mudar o nome do worktree" dialog. */
export function RenameWorktree(props: { current: string; value: string; error: string; onInput: (v: string) => void; onClose: () => void; onSubmit: () => void }) {
  return (
    <Modal label="Mudar o nome do worktree" dirty={props.value.trim() !== props.current} onClose={props.onClose}>
      <div class="m-title font-semibold">Mudar o nome do worktree</div>
      <Field label="Nome" full><Input
          value={props.value}
          onInput={(e) => props.onInput(e.currentTarget.value)}
          onKeyDown={(e) => e.key === "Enter" && props.onSubmit()}
          ref={(el) => queueMicrotask(() => { el.focus(); el.select(); })}
        /></Field>
      <Show when={props.error}>
        <div class="note-status bad" role="alert">{props.error}</div>
      </Show>
      <div class="m-actions">
        <Button onClick={props.onClose}>Cancelar</Button>
        <Button variant="default" disabled={!props.value.trim() || props.value.trim() === props.current} onClick={props.onSubmit}>
          Mudar o nome
        </Button>
      </div>
    </Modal>
  );
}
