import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { diagnostics } from "./api";
import { restartDaemonFlow } from "./daemonActions";
import { onTauriEvent } from "./tauriEvents";
import { Button } from "./ui";
import { S } from "./strings";
import { buildLabel, staleInfo, type StaleInfo } from "./skewLogic";

// the banner lives inside the shared toast stack (ErrorToast), so it flows there instead of floating on its own
/**
 * "Daemon older than the app": shown while the running daemon is of an older release than this app, or the host kept an
 * older daemon on purpose when it connected (`daemon_offer`: work was running, or the build could not be confirmed).
 * Restarting goes through the usual replace path; agents resume from their saved conversation and terminals come back,
 * which the confirmation says. "Mais tarde" hides it until the app starts again, even if the host reports it again.
 */
export function VersionSkew() {
  const [info, setInfo] = createSignal<StaleInfo | null>(null);
  const [dismissed, setDismissed] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const check = () =>
    diagnostics().then(
      (d) => setInfo(staleInfo(d)),
      () => undefined,
    );
  onMount(() => {
    // the daemon only changes on a connect: look when the app opens and whenever the host says it (re)connected
    void check();
    const offs = ["daemon-stale", "daemon-restarted", "daemon-reconnected"].map((e) =>
      onTauriEvent(e, () => void check()),
    );
    onCleanup(() => offs.forEach((off) => off()));
  });
  return (
    <Show when={!dismissed() && info()?.show && info()}>
      {(i) => (
        <div class="error-toast notice flow" role="status" data-testid="version-skew">
          <span class="et-msg">
            {S.versionSkew.text(
              buildLabel(i().app, S.versionSkew.unknown),
              buildLabel(i().daemon, S.versionSkew.unknown),
            )}
            <Show when={i().reason}>{(r) => " " + S.versionSkew[r()]}</Show>
          </span>
          <Button
            size="sm"
            class="flex-none"
            disabled={busy()}
            onClick={() => void restartDaemonFlow(setBusy, async () => void (await check()), S.versionSkew.confirm)}
          >
            {busy() ? S.restartingDaemon : S.versionSkew.restart}
          </Button>
          <Button size="sm" class="flex-none" onClick={() => setDismissed(true)}>
            {S.update.later}
          </Button>
        </div>
      )}
    </Show>
  );
}
