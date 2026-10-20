import { Show } from "solid-js";
import { error, setError } from "./store";

/** Failures from any action; stays until dismissed. */
export function ErrorToast() {
  return (
    <Show when={error()}>
      <div class="error-toast" role="alert">
        <span class="et-msg">{error()}</span>
        <button class="icon" title="Fechar" onClick={() => setError("")}>
          ×
        </button>
      </div>
    </Show>
  );
}
