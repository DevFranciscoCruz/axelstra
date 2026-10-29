import { onDaemonEvent } from "../api";
import { wakeFailedText } from "../hibernateLogic";
import { H } from "../strings.hibernate";
import { L } from "../strings.logic";
import { toast } from "../toasts";
import { sessions } from "./sessions";

const nameOf = (id: unknown): string => {
  const n = typeof id === "number" ? id : -1;
  return sessions().find((s) => s.id === n)?.label ?? L.session(n);
};

/** Notices of hibernation. The sessions list itself refreshes on `session.changed`, which the daemon sends with each event;
 *  here only what the user must be told: a session that could not wake up, and keys typed into a sleeper (lost). */
export const watchHibernate = (): (() => void) => {
  const offs = [
    onDaemonEvent("session.wake_failed", (d) => {
      // a bare payload is the stream reconnecting: nothing happened
      if (d.id === undefined) return;
      toast({ text: wakeFailedText(nameOf(d.id), d), kind: "error", key: `wake:${String(d.id)}` });
    }),
    onDaemonEvent("session.input_dropped", (d) => {
      if (d.id === undefined) return;
      toast({ text: H.notice.inputDropped(nameOf(d.id)), kind: "info", key: `dropped:${String(d.id)}`, record: false });
    }),
  ];
  return () => offs.forEach((off) => off());
};
