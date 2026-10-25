import { Show, createSignal } from "solid-js";
import { pickFolder } from "../api";
import { AP } from "../strings.sources";
import { Button, Input } from "../ui";

/** A folder on this computer: typed, or chosen in the native dialog (the Rust side returns the path). */
export function FolderField(props: {
  id: string;
  label: string;
  value: string;
  onChange: (v: string) => void;
  error?: string;
  disabled?: boolean;
}) {
  const [note, setNote] = createSignal("");
  const browse = async () => {
    setNote("");
    try {
      const p = await pickFolder(AP.folderPickTitle);
      if (p) props.onChange(p);
    } catch (e) {
      console.debug("folder_pick", e);
      setNote(AP.folderPickFailed);
    }
  };
  return (
    <div class="flex min-w-0 flex-col gap-1 text-xs text-muted-foreground">
      <label for={props.id}>{props.label}</label>
      <div class="flex items-center gap-2">
        <Input
          id={props.id}
          value={props.value}
          placeholder={AP.folderPh}
          autocomplete="off"
          spellcheck={false}
          disabled={props.disabled}
          aria-invalid={!!props.error}
          aria-describedby={`${props.id}-msg`}
          onInput={(e) => {
            setNote("");
            props.onChange(e.currentTarget.value);
          }}
        />
        <Button type="button" disabled={props.disabled} onClick={() => void browse()}>
          {AP.folderBrowse}
        </Button>
      </div>
      <div id={`${props.id}-msg`} class="auto-note text-[var(--err)]" role="alert">
        <Show when={props.error || note()}>{(m) => m()}</Show>
      </div>
    </div>
  );
}
