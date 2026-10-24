import { Show } from "solid-js";
import { Modal } from "../Modal";
import { Button, Field, Input } from "../ui";
import { S } from "../strings";
import { U } from "../strings.ui";

/** The "Mudar o nome do worktree" dialog. */
export function RenameWorktree(props: {
  current: string;
  value: string;
  error: string;
  onInput: (v: string) => void;
  onClose: () => void;
  onSubmit: () => void;
}) {
  return (
    <Modal label={U.renameWorktree.title} dirty={props.value.trim() !== props.current} onClose={props.onClose}>
      <div class="m-title font-semibold">{U.renameWorktree.title}</div>
      <Field label={U.renameWorktree.name} full>
        <Input
          value={props.value}
          onInput={(e) => props.onInput(e.currentTarget.value)}
          onKeyDown={(e) => e.key === "Enter" && props.onSubmit()}
          ref={(el) =>
            queueMicrotask(() => {
              el.focus();
              el.select();
            })
          }
        />
      </Field>
      <Show when={props.error}>
        <div class="note-status bad" role="alert">
          {props.error}
        </div>
      </Show>
      <div class="m-actions">
        <Button onClick={props.onClose}>{S.cancel}</Button>
        <Button
          variant="default"
          disabled={!props.value.trim() || props.value.trim() === props.current}
          onClick={props.onSubmit}
        >
          {U.renameWorktree.submit}
        </Button>
      </div>
    </Modal>
  );
}
