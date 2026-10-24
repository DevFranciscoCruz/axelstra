import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { Button } from "./ui";
import { createPoller } from "./poller";
import { errorText } from "./appError";
import { S } from "./strings";
import { readString, writeString } from "./persist";

/** the version the user said "later" to: remembered, so a restart does not bring the same card back */
const DISMISSED_KEY = "axelstra.updateDismissed";
const loadDismissed = () => readString(DISMISSED_KEY);
const saveDismissed = (v: string) => writeString(DISMISSED_KEY, v); // storage unavailable: it just comes back next start
// the card lives inside the shared toast stack (ErrorToast), so it flows there instead of floating on its own

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
  const [dismissed, setDismissed] = createSignal<string | null>(loadDismissed());
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
    } catch (e) {
      // offline or no release published yet: stay quiet, retry later
      console.debug("update_check", e);
    }
    lastCheck = Date.now();
  };

  onMount(() => {
    const first = setTimeout(() => void check(), 5000);
    const again = createPoller(check, CHECK_INTERVAL_MS, { visibleOnly: false });
    let unlisten: (() => void) | undefined;
    let gone = false;
    void getCurrentWindow()
      // eslint-disable-next-line solid/reactivity -- focus event handler
      .onFocusChanged(({ payload: focused }) => {
        if (focused && Date.now() - lastCheck > FOCUS_THROTTLE_MS) void check();
      })
      .then((fn) => (gone ? fn() : (unlisten = fn)))
      .catch((e) => console.debug("onFocusChanged", e));
    onCleanup(() => {
      gone = true;
      clearTimeout(first);
      again.stop();
      unlisten?.();
    });
  });

  const install = async () => {
    setBusy(true);
    setErr("");
    try {
      await invoke("update_apply");
    } catch (e) {
      setErr(errorText(e));
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
        <div class="update-toast flow" role="status" aria-label={S.update.available}>
          <div class="ut-head">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M8 2.5v8M4.8 7.6 8 10.8l3.2-3.2M3 13h10" />
            </svg>
            <div>
              <p class="ut-title">{S.update.ready(i().version)}</p>
              <p class="ut-sub">{S.update.sub}</p>
            </div>
          </div>
          <div class="ut-actions">
            <Button
              disabled={busy()}
              onClick={() => {
                saveDismissed(i().version);
                setDismissed(i().version);
              }}
            >
              {S.update.later}
            </Button>
            <Button variant="default" disabled={busy()} onClick={() => void install()}>
              {busy() ? S.update.restarting : S.update.restart}
            </Button>
          </div>
          <Show when={i().notes}>
            {(n) => (
              <details class="mt-2 text-[length:var(--fs-md)]">
                <summary class="cursor-pointer text-muted-foreground">{S.update.notes}</summary>
                <p class="mt-1 max-h-32 overflow-auto whitespace-pre-line">{n()}</p>
              </details>
            )}
          </Show>
          <Show when={err()}>
            <div class="error">{err()}</div>
          </Show>
        </div>
      )}
    </Show>
  );
}
