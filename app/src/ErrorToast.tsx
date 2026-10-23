import { Show } from "solid-js";
import { Icon } from "./Icon";
import { daemonReconnecting, error, notice, setError, setNoticeSig } from "./store";

// .error-toast is position:fixed on its own; inside the stack the wrapper positions, the toasts flow
const FLOW = { position: "static", transform: "none" } as const;

/** Failures from any action stay until dismissed; plain notices go away by themselves. Both show at once, stacked (at most one of each). */
export function ErrorToast() {
  // the stack is a polite live region that is always on the page, so a notice that appears is announced
  return (
    <div aria-live="polite" aria-atomic="false">
    <Show when={error() || notice() || daemonReconnecting()}>
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
        }}
      >
        <Show when={daemonReconnecting()}>
          <div class="error-toast notice" role="status" style={FLOW}>
            <span class="et-msg">A reconectar ao daemon…</span>
          </div>
        </Show>
        <Show when={notice()}>
          <div class="error-toast notice" role="status" style={FLOW}>
            <span class="et-msg">{notice()}</span>
            <button class="icon" title="Fechar" aria-label="Fechar" onClick={() => setNoticeSig("")}>
              <Icon name="x" />
            </button>
          </div>
        </Show>
        <Show when={error()}>
          <div class="error-toast" role="alert" style={FLOW}>
            <span class="et-msg">{error()}</span>
            <button class="icon" title="Fechar" aria-label="Fechar" onClick={() => setError("")}>
              <Icon name="x" />
            </button>
          </div>
        </Show>
      </div>
    </Show>
    </div>
  );
}
