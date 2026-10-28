import { For, Show } from "solid-js";
import { AddHostForm } from "./AddHostForm";
import { ask } from "./Confirm";
import { HostBadge, reconnectWithNotice } from "./HostBadge";
import { errorText } from "./appError";
import { coded } from "./msg";
import { canReconnect } from "./hostLogic";
import { hosts, removeHost, setError, showNotice } from "./store";
import { S } from "./strings";
import { R } from "./strings.remote";
import { Button } from "./ui";

/** Settings block: the remote hosts with their state, reconnect and remove, and a form to add one. */
export function HostsSection() {
  const drop = async (id: string, name: string) => {
    const ok = await ask(R.removeAsk(name), {
      title: R.removeTip(name),
      kind: "danger",
      okLabel: R.remove,
      cancelLabel: S.cancel,
    });
    if (!ok) return;
    try {
      await removeHost(id);
      showNotice(R.removed(name));
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <section class="set-sec" aria-label={R.section}>
      <h4>{R.section}</h4>
      <p class="auto-note">{R.sectionNote}</p>
      <Show when={hosts().length > 0} fallback={<div class="auto-note">{R.none}</div>}>
        <ul class="m-0 flex list-none flex-col gap-1.5 p-0" aria-label={R.section}>
          <For each={hosts()}>
            {(h) => (
              <li class="flex flex-wrap items-center gap-2 rounded-md border border-border px-2 py-1.5">
                <strong class="text-base">{h.name}</strong>
                <code class="text-xs text-muted-foreground">{h.spec}</code>
                <span class="grow" />
                <HostBadge host={h.id} />
                <Show when={h.state === "ok" && R.builds(h.proto, h.build)}>
                  {(b) => <span class="text-xs text-muted-foreground">{b()}</span>}
                </Show>
                <Show when={canReconnect(h.state)}>
                  <Button
                    type="button"
                    size="sm"
                    aria-label={R.reconnectTip(h.name)}
                    onClick={() => void reconnectWithNotice(h.id)}
                  >
                    {R.reconnect}
                  </Button>
                </Show>
                <Button
                  type="button"
                  size="sm"
                  aria-label={R.removeTip(h.name)}
                  onClick={() => void drop(h.id, h.name)}
                >
                  {R.remove}
                </Button>
                <Show when={h.error && h.state !== "ok"}>
                  <p class="auto-note m-0 w-full break-words text-[var(--danger)]">
                    {coded(h.error_code, h.error_params, h.error ?? "")}
                  </p>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <AddHostForm />
    </section>
  );
}
