import { Show, createSignal, onCleanup } from "solid-js";
import { Icon } from "../Icon";
import { clipboardWrite } from "../api";
import { P } from "../strings.panels";
import { Button } from "./button";

/** the text the user must type, in a box with a copy button (like Cloudflare's delete confirmation) */
export function CopyWord(props: { text: string }) {
  const [done, setDone] = createSignal(false);
  let timer: number | undefined;
  onCleanup(() => window.clearTimeout(timer));
  const copy = () =>
    void clipboardWrite(props.text).then(
      () => {
        setDone(true);
        window.clearTimeout(timer);
        timer = window.setTimeout(() => setDone(false), 1500);
      },
      () => undefined,
    );
  return (
    <div class="flex items-center gap-2 rounded-sm border border-input bg-[var(--s-in)] py-1 pl-2 pr-1">
      <code class="min-w-0 flex-1 select-all break-all font-mono text-sm text-foreground">{props.text}</code>
      <Button
        variant="icon"
        size="icon"
        aria-label={done() ? P.confirm.copied : P.confirm.copy}
        title={done() ? P.confirm.copied : P.confirm.copy}
        onClick={copy}
      >
        <Show when={done()} fallback={<Icon name="copy" size={14} />}>
          <Icon name="check" size={14} />
        </Show>
      </Button>
    </div>
  );
}
