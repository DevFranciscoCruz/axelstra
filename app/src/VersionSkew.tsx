import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { diagnostics } from "./api";
import { restartDaemonFlow } from "./daemonActions";
import { createPoller } from "./poller";
import { onTauriEvent } from "./tauriEvents";
import { Button } from "./ui";
import { S } from "./strings";
import { daemonOlderThanApp } from "./skewLogic";

const CHECK_MS = 60_000;
// the banner lives inside the shared toast stack (ErrorToast), so it flows there instead of floating on its own
const FLOW = { position: "static", transform: "none" } as const;

/**
 * "Daemon older than the app": shown while the running daemon is of an older release than this app. Restarting is safe,
 * terminals and agents come back through session restore, so the button restarts it without asking twice. "Mais tarde"
 * hides it until the app starts again.
 */
export function VersionSkew() {
  const [old, setOld] = createSignal(false);
  const [dismissed, setDismissed] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const check = () =>
    diagnostics().then(
      (d) => setOld(daemonOlderThanApp(d)),
      () => undefined,
    );
  onMount(() => {
    const poll = createPoller(check, CHECK_MS);
    const off = onTauriEvent("daemon-restarted", () => void check());
    onCleanup(() => {
      poll.stop();
      off();
    });
  });
  return (
    <Show when={old() && !dismissed()}>
      <div class="error-toast notice" role="status" data-testid="version-skew" style={FLOW}>
        <span class="et-msg">{S.versionSkew.text}</span>
        <Button
          size="sm"
          class="flex-none"
          disabled={busy()}
          onClick={() => void restartDaemonFlow(setBusy, async () => void (await check()))}
        >
          {busy() ? S.restartingDaemon : S.versionSkew.restart}
        </Button>
        <Button size="sm" class="flex-none" onClick={() => setDismissed(true)}>
          {S.update.later}
        </Button>
      </div>
    </Show>
  );
}
