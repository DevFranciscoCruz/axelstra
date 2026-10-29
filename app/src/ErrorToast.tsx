import { For, Show, type JSX } from "solid-js";
import { Icon } from "./Icon";
import { daemonReconnecting, dismissToast, retryDaemon, toasts } from "./store";
import { pauseToast, resumeToast, type Toast } from "./toasts";
import { Button } from "./ui";
import { cancelSlow, slowMethods } from "./slowCalls";
import { S } from "./strings";

// .error-toast is position:fixed on its own; inside the stack the wrapper positions, the toasts flow
function ToastCard(props: { t: Toast }) {
  const t = () => props.t;
  // a toast that closes by itself waits while the pointer or the keyboard focus is on it (WCAG 2.2.1)
  return (
    <div
      class="error-toast flow"
      classList={{ notice: t().kind !== "error" }}
      data-kind={t().kind}
      onMouseEnter={() => pauseToast(t().id)}
      onMouseLeave={() => resumeToast(t().id)}
      onFocusIn={() => pauseToast(t().id)}
      onFocusOut={() => resumeToast(t().id)}
    >
      <span class="et-msg">{t().text}</span>
      <Show when={t().action}>
        {(a) => (
          <Button
            size="sm"
            class="flex-none"
            onClick={() => {
              dismissToast(t().id);
              a().run();
            }}
          >
            {a().label}
          </Button>
        )}
      </Show>
      <Button variant="icon" size="icon" class="size-6 flex-none" title={S.toast.close} aria-label={S.toast.close} onClick={() => dismissToast(t().id)}>
        <Icon name="x" />
      </Button>
    </div>
  );
}

/**
 * The one place toasts appear, stacked above the status bar: failures (kept until closed), notices (closing by
 * themselves), the reconnection banner, and whatever else is passed as children (the update card), so none of
 * them lands on top of another. Screen readers get two small always-present live regions (status and alert)
 * whose text changes; the visible cards are not live regions themselves, which announced unreliably.
 */
/** the newest toast of these kinds, as a one-item list: `<For>` rebuilds the node for a new toast, so a second toast with the same text is announced again */
const latestOf = (...kinds: Toast["kind"][]) => {
  const t = [...toasts()].reverse().find((x) => kinds.includes(x.kind));
  return t ? [t] : [];
};

export function ErrorToast(props: { children?: JSX.Element }) {
  return (
    <>
      <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">
        {daemonReconnecting() ? S.toast.reconnecting : <For each={latestOf("info", "warn")}>{(t) => t.text}</For>}
      </div>
      <div class="sr-only" role="alert" aria-atomic="true">
        <For each={latestOf("error")}>{(t) => t.text}</For>
      </div>
      <div class="toast-stack">
        <div class="toast-stack-in">
          <Show when={daemonReconnecting()}>
            <div class="error-toast notice flow">
              <span class="et-msg">{S.toast.reconnecting}</span>
              <Button size="sm" class="flex-none" onClick={() => void retryDaemon()}>
                {S.toast.retryNow}
              </Button>
            </div>
          </Show>
          <For each={slowMethods().slice(0, 3)}>
            {(m) => (
              <div class="error-toast notice flow" role="status">
                <span class="et-msg">{S.toast.slow(m)}</span>
                <Button size="sm" class="flex-none" onClick={() => cancelSlow(m)}>
                  {S.toast.slowCancel}
                </Button>
              </div>
            )}
          </For>
          <For each={toasts()}>{(t) => <ToastCard t={t} />}</For>
          {props.children}
        </div>
      </div>
    </>
  );
}
