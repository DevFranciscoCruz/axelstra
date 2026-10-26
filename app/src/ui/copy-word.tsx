import { Show, createSignal, onCleanup } from "solid-js";
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
        <Show
          when={done()}
          fallback={
            <svg
              width="14"
              height="14"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              stroke-width="1.4"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
              <path d="M10.5 5.5V3.5a1.5 1.5 0 0 0-1.5-1.5H3.5A1.5 1.5 0 0 0 2 3.5V9a1.5 1.5 0 0 0 1.5 1.5h2" />
            </svg>
          }
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="m3 8.5 3.2 3.2L13 4.5" />
          </svg>
        </Show>
      </Button>
    </div>
  );
}
