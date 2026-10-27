import { call, daemonShutdown } from "./api";
import { ask } from "./Confirm";
import { errorText, waitUntil } from "./settingsLogic";
import { showNotice } from "./store";
import { S } from "./strings";

/**
 * Restart the daemon after asking: it is a separate process that outlives the app, so shutting it down is safe on every
 * platform; the next call starts a fresh one and the app restores the sessions (see "daemon-restarted").
 * `onState` follows the progress (true while restarting); `after` runs once the new daemon answers. Returns false when the
 * user declined or it failed.
 */
export async function restartDaemonFlow(
  onState?: (restarting: boolean) => void,
  after?: () => void | Promise<void>,
): Promise<boolean> {
  if (!(await ask(S.restartConfirm, { title: S.restartTitle, kind: "warning", okLabel: S.restartDaemon })))
    return false;
  onState?.(true);
  try {
    await daemonShutdown().catch((e) => console.debug("daemon.shutdown", e)); // the connection drops with the daemon
    if (!(await waitUntil(() => call("ping"), 20))) throw new Error(S.daemonSilent);
    await after?.();
    return true;
  } catch (e) {
    showNotice(S.restartFailed + errorText(e));
    return false;
  } finally {
    onState?.(false);
  }
}
