import type { WaitInfo } from "./api";
import { L } from "./strings.logic";

export type WaitDecision = {
  /** marker of the wait episode now in force; keep it for the next call (undefined: no episode) */
  seen: string | undefined;
  /** line for the bell and the system notification; null when nothing is announced */
  text: string | null;
  /** whether the system notification goes out too (the bell entry is recorded whenever `text` is set) */
  os: boolean;
};

/** An agent that asks for permission or a question is announced once per episode, from the agent's own event text.
 *  `idle_input` (the agent is just at its prompt) never is. `grace`: the app just started, so a restored wait is only
 *  remembered. `watching`: the user is looking at the worktree with focus, so only the bell entry is recorded. */
export const decideWait = (
  seen: string | undefined,
  wait: WaitInfo | null | undefined,
  who: string,
  worktree: string,
  opts: { grace: boolean; watching: boolean },
): WaitDecision => {
  if (!wait || wait.kind === "idle_input") return { seen: undefined, text: null, os: false };
  const key = `${wait.kind}\n${wait.text}`;
  if (key === seen || opts.grace) return { seen: key, text: null, os: false };
  const t = wait.text.trim();
  const T = L.notify.wait;
  const text =
    wait.kind === "permission"
      ? t
        ? T.permission(who, worktree, t)
        : T.permissionGeneric(who, worktree)
      : t
        ? T.question(who, worktree, t)
        : T.questionGeneric(who, worktree);
  return { seen: key, text, os: !opts.watching };
};
