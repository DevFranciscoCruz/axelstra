import { createSignal } from "solid-js";
import { friendlyError } from "../friendly";
import { CANCELLED } from "../appError";
import { msgText } from "../msg";
import { dismissKey, dismissPrefix, latestText, toast } from "../toasts";

/** The newest failure on screen ("" when none). Failures live in the toast list, one per key (see toasts.ts). */
export const error = () => latestText("error");
/** the app is reconnecting to the daemon (it was replaced or crashed): the UI says so until it is back */
export const [daemonReconnecting, setDaemonReconnecting] = createSignal(false);
/** a short informational message (not a failure): shown for a few seconds */
export const notice = () => latestText("info");
export const showNotice = (m: string) => void toast({ text: m, kind: "info" });

const ACTION_KEY = "action:";
/** Shows a failure. Two different failures stack; the same one again replaces itself. `setError("")` closes the action failures.
 *  `key` ties it to a poll, which clears only its own with `clearError(key)`. */
export const setError = (m: string, key?: string) => {
  if (!m) return dismissPrefix(ACTION_KEY);
  if (m === msgText(CANCELLED)) return; // the user cancelled a slow request: no failure to report
  const text = friendlyError(m);
  toast({ text, kind: "error", key: key ?? `${ACTION_KEY}${text}`, record: !key });
};
/** a poll that works again takes down the failure it raised, and only that one */
export const clearError = (key: string) => dismissKey(key);
