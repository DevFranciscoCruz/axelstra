import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { Button } from "./ui";

/** "A carregar…" that tells assistive tech it is busy, and offers a retry when the wait drags on */
export function Loading(props: { onRetry?: () => void; timeoutMs?: number; class?: string }) {
  const [slow, setSlow] = createSignal(false);
  onMount(() => {
    const t = setTimeout(() => setSlow(true), props.timeoutMs ?? 10000);
    onCleanup(() => clearTimeout(t));
  });
  return (
    <div class={props.class ?? "placeholder"} role="status" aria-busy="true">
      <span>A carregar…</span>
      <Show when={slow()}>
        <div class="auto-note">Está a demorar mais do que o normal.</div>
        <Show when={props.onRetry}>
          <Button size="sm" onClick={() => props.onRetry?.()}>
            Tentar de novo
          </Button>
        </Show>
      </Show>
    </div>
  );
}
