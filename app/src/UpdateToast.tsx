import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { Button } from "./ui";

type Info = { version: string; notes: string | null };

// check at launch, every 6 h and when the window regains focus (throttled); never install without a click
const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
const FOCUS_THROTTLE_MS = 5 * 60 * 1000;

/**
 * A new release, announced as a card floating over the corner of whatever is open. It is downloaded
 * in the background first and the card only appears once it is ready to install, so "Reiniciar
 * agora" never makes you wait. It does not fade on a timer: the only ways out are its two buttons.
 * Dismissed or not, a downloaded update is installed when the app is closed.
 */
export function UpdateToast() {
  const [info, setInfo] = createSignal<Info | null>(null);
  const [ready, setReady] = createSignal<string | null>(null);
  const [dismissed, setDismissed] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [err, setErr] = createSignal("");
  let lastCheck = 0;

  const check = async () => {
    try {
      const found = await invoke<Info | null>("update_check");
      setInfo(found);
      if (found && ready() !== found.version) {
        await invoke("update_download", { expected: found.version });
        setReady(found.version);
      }
    } catch {
      /* offline or no release published yet: stay quiet, retry later */
    }
    lastCheck = Date.now();
  };

  onMount(() => {
    const first = setTimeout(() => void check(), 5000);
    const again = setInterval(() => void check(), CHECK_INTERVAL_MS);
    let unlisten: (() => void) | undefined;
    let gone = false;
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (focused && Date.now() - lastCheck > FOCUS_THROTTLE_MS) void check();
      })
      .then((fn) => (gone ? fn() : (unlisten = fn)))
      .catch(() => {});
    onCleanup(() => {
      gone = true;
      clearTimeout(first);
      clearInterval(again);
      unlisten?.();
    });
  });

  const install = async () => {
    setBusy(true);
    setErr("");
    try {
      await invoke("update_apply");
    } catch (e) {
      setErr(String(e));
      setBusy(false);
    }
  };

  const shown = () => {
    const i = info();
    return i && i.version === ready() && i.version !== dismissed() ? i : null;
  };

  return (
    <Show when={shown()}>
      {(i) => (
        <div class="update-toast" role="status" aria-label="Atualização disponível">
          <div class="ut-head">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M8 2.5v8M4.8 7.6 8 10.8l3.2-3.2M3 13h10" />
            </svg>
            <div>
              <p class="ut-title">Versão {i().version} pronta</p>
              <p class="ut-sub">Os terminais e agentes continuam a correr. Se preferires, a atualização instala ao fechar a app.</p>
            </div>
          </div>
          <div class="ut-actions">
            <Button disabled={busy()} onClick={() => setDismissed(i().version)}>
              Mais tarde
            </Button>
            <Button variant="default" disabled={busy()} onClick={() => void install()}>
              {busy() ? "A reiniciar…" : "Reiniciar agora"}
            </Button>
          </div>
          <Show when={err()}>
            <div class="error">{err()}</div>
          </Show>
        </div>
      )}
    </Show>
  );
}
