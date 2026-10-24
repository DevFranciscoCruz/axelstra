import { For, Show, type JSX } from "solid-js";
import { Icon } from "./Icon";
import { daemonReconnecting, dismissToast, retryDaemon, toasts } from "./store";
import { latestText, pauseToast, resumeToast, type Toast } from "./toasts";
import { Button } from "./ui";

// .error-toast is position:fixed on its own; inside the stack the wrapper positions, the toasts flow
const FLOW = { position: "static", transform: "none" } as const;

function ToastCard(props: { t: Toast }) {
  const t = () => props.t;
  // a toast that closes by itself waits while the pointer or the keyboard focus is on it (WCAG 2.2.1)
  return (
    <div
      class="error-toast"
      classList={{ notice: t().kind !== "error" }}
      data-kind={t().kind}
      style={FLOW}
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
      <Button variant="icon" size="icon" class="size-6 flex-none" title="Fechar" aria-label="Fechar" onClick={() => dismissToast(t().id)}>
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
export function ErrorToast(props: { children?: JSX.Element }) {
  return (
    <>
      <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">
        {daemonReconnecting() ? "A reconectar ao daemon…" : latestText("info") || latestText("warn")}
      </div>
      <div class="sr-only" role="alert" aria-atomic="true">
        {latestText("error")}
      </div>
      <div
        style={{
          position: "fixed",
          left: "50%",
          bottom: "36px", // above the status bar (24px)
          transform: "translateX(-50%)",
          "z-index": 60,
          display: "flex",
          "flex-direction": "column",
          gap: "8px",
          "align-items": "stretch",
          "max-width": "min(560px, 92vw)",
          // the empty stack must not catch clicks meant for the app below it
          "pointer-events": "none",
        }}
      >
        <div style={{ display: "contents", "pointer-events": "auto" }}>
          <Show when={daemonReconnecting()}>
            <div class="error-toast notice" style={FLOW}>
              <span class="et-msg">A reconectar ao daemon…</span>
              <Button size="sm" class="flex-none" onClick={() => void retryDaemon()}>
                Tentar agora
              </Button>
            </div>
          </Show>
          <For each={toasts()}>{(t) => <ToastCard t={t} />}</For>
          {props.children}
        </div>
      </div>
    </>
  );
}
