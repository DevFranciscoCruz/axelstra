import { Show } from "solid-js";
import type { HostState } from "./api";
import { R } from "./strings.remote";
import { Button, cn } from "./ui";
import { hostById, reconnectHost } from "./store/hosts";
import { setError, showNotice } from "./store";
import { errorText } from "./appError";

/** shape + colour: the state never depends on colour alone */
const GLYPH: Record<HostState, string> = { idle: "○", connecting: "◌", ok: "●", lost: "✕", auth: "!" };
const TONE: Record<HostState, string> = {
  idle: "text-muted-foreground",
  connecting: "text-[var(--warn)]",
  ok: "text-[var(--ok)]",
  lost: "text-[var(--danger)]",
  auth: "text-[var(--warn)]",
};

/** Reconnects a host and tells the user how it went (the badge and the Settings list share it). */
export const reconnectWithNotice = async (id: string) => {
  const h = hostById(id);
  const name = h?.name ?? id;
  try {
    await reconnectHost(id);
    showNotice(R.reconnected(name));
  } catch (e) {
    setError(R.reconnectFailed(name, errorText(e).slice(0, 200)));
  }
};

/** The host name of a remote project with its connection-state dot, and "Voltar a ligar" when the link is down. */
export function HostBadge(props: { host: string; compact?: boolean }) {
  const h = () => hostById(props.host);
  const state = (): HostState => h()?.state ?? "idle";
  const name = () => h()?.name ?? props.host;
  return (
    <span class="inline-flex min-w-0 flex-none items-center gap-1">
      <span
        class="inline-flex max-w-[110px] items-center gap-1 overflow-hidden rounded-xl border border-[var(--line-2)] bg-secondary px-1.5 py-[2px] text-[length:var(--fs-xs)] leading-none text-muted-foreground"
        title={R.badgeTip(name(), h()?.spec ?? "", state())}
      >
        <span class={cn("flex-none", TONE[state()], state() === "connecting" && "animate-pulse")} aria-hidden="true">
          {GLYPH[state()]}
        </span>
        <span class="truncate">{name()}</span>
      </span>
      <span class="sr-only" role="status" aria-live="polite" aria-atomic="true">
        {R.stateOf(name(), state())}
      </span>
      <Show when={state() === "lost" || state() === "auth"}>
        <Button
          type="button"
          variant="chip"
          size="chip"
          class="py-px"
          title={R.reconnectTip(name())}
          aria-label={R.reconnectTip(name())}
          onClick={() => void reconnectWithNotice(props.host)}
        >
          {R.reconnect}
        </Button>
      </Show>
    </span>
  );
}
