import { Show } from "solid-js";
import { Icon } from "./Icon";
import { daemonReconnecting, error, notice, setError, setNoticeSig } from "./store";
import { Button } from "./ui";

// .error-toast is position:fixed on its own; inside the stack the wrapper positions, the toasts flow
const FLOW = { position: "static", transform: "none" } as const;

/** Failures from any action stay until dismissed; plain notices go away by themselves. Both show at once, stacked (at most one of each). */
export function ErrorToast() {
  // the stack is a polite live region that is always on the page, so a notice that appears is announced; fixed, so the
  // empty region is not a cell of the app grid (it pushed the sidebar one column over)
  return (
    <div aria-live="polite" aria-atomic="false" style={{ position: "fixed", left: 0, top: 0, width: 0, height: 0 }}>
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
            <Button variant="icon" size="icon" class="size-6 flex-none" title="Fechar" aria-label="Fechar" onClick={() => setNoticeSig("")}>
              <Icon name="x" />
            </Button>
          </div>
        </Show>
        <Show when={error()}>
          <div class="error-toast" role="alert" style={FLOW}>
            <span class="et-msg">{error()}</span>
            <Button variant="icon" size="icon" class="size-6 flex-none" title="Fechar" aria-label="Fechar" onClick={() => setError("")}>
              <Icon name="x" />
            </Button>
          </div>
        </Show>
      </div>
    </Show>
    </div>
  );
}
