import { createSignal, untrack } from "solid-js";
import { memStatus, onDaemonEvent } from "../api";
import { pushNotification } from "../notifications";
import { G } from "../strings.memory";
import { toast } from "../toasts";
import {
  applyPaused,
  applyPressure,
  applyResumed,
  applyStatus,
  initialMem,
  levelTransition,
  pausedDelta,
  type MemState,
} from "../memoryLogic";

/** Memory pressure and what the governor froze, fed by the watch-stream events; `mem.status` fills it in at start and after a reconnect. */
export const [memory, setMemory] = createSignal<MemState>(initialMem());

const LEVEL_KEY = "memory:level";

/** Resident memory of a session in MB, when the daemon listed it among the biggest. */
export const rssOf = (id: number): number | undefined => memory().top.find((t) => t.session === id)?.rss_mb;

/** Replaces the state and tells the user what changed: a level change, and sessions the governor froze or resumed. */
const move = (next: MemState) => {
  const prev = memory();
  setMemory(next);
  const level = levelTransition(prev, next);
  if (level) {
    const text = G.notify.level(level, next.free_mb);
    toast({ text, kind: level === "critical" ? "error" : level === "high" ? "warn" : "info", key: LEVEL_KEY });
    if (level !== "normal") pushNotification({ kind: "attention", worktree: "", text });
  }
  if (!prev.loaded) return;
  const d = pausedDelta(prev, next);
  const frozen = d.paused.filter((p) => p.reason === "memory");
  if (frozen.length) {
    const text = G.notify.paused(frozen.length);
    toast({ text, kind: "warn", key: "memory:paused" });
    pushNotification({ kind: "attention", worktree: "", text });
  }
  const back = d.resumed.filter((p) => p.reason === "memory");
  if (back.length) toast({ text: G.notify.resumed(back.length), kind: "info", key: "memory:resumed" });
};

/** Reads `mem.status` (an older daemon has none: the state stays empty and nothing is shown). */
export const refreshMemory = () =>
  memStatus().then(
    (st) => move(applyStatus(memory(), st)),
    (e) => console.debug("mem.status", e),
  );

/** A listener for one event: a bare payload (the stream reconnected) reads the status again, a real one goes through `reduce`. */
const feed = (reduce: (s: MemState, d: Record<string, unknown>) => MemState) => (d: Record<string, unknown>) => {
  if (Object.keys(d).length === 0) void refreshMemory();
  else move(reduce(untrack(memory), d));
};

/** Starts listening; returns the function that stops. Called once by the bootstrap. No timer: the daemon pushes the events,
 *  and a reconnect of the stream fires each of them once with an empty payload, which is the cue to read the status again. */
export const watchMemory = (): (() => void) => {
  void refreshMemory();
  const offs = [
    onDaemonEvent("memory.pressure", feed(applyPressure)),
    onDaemonEvent(
      "session.paused",
      feed((s, d) => applyPaused(s, d, Date.now())),
    ),
    onDaemonEvent("session.resumed", feed(applyResumed)),
  ];
  return () => offs.forEach((off) => off());
};
