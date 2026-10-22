import { Show } from "solid-js";
import { Icon } from "./Icon";
import { error, notice, setError, setNoticeSig } from "./store";

/** Failures from any action stay until dismissed; plain notices go away by themselves. */
export function ErrorToast() {
  return (
    <>
      <Show when={error()}>
        <div class="error-toast" role="alert">
          <span class="et-msg">{error()}</span>
          <button class="icon" title="Fechar" onClick={() => setError("")}>
            <Icon name="x" />
          </button>
        </div>
      </Show>
      <Show when={!error() && notice()}>
        <div class="error-toast notice" role="status">
          <span class="et-msg">{notice()}</span>
          <button class="icon" title="Fechar" onClick={() => setNoticeSig("")}>
            <Icon name="x" />
          </button>
        </div>
      </Show>
    </>
  );
}
