import { coded } from "./msg";
import { H } from "./strings.hibernate";

/** `session.wake_failed {id, code, params, error}` as a sentence: the reason in the app's language when the code is known. */
export const wakeFailedText = (name: string, d: Record<string, unknown>): string =>
  H.notice.wakeFailed(
    name,
    coded(
      typeof d.code === "string" ? d.code : undefined,
      (d.params ?? undefined) as Record<string, unknown> | undefined,
      typeof d.error === "string" ? d.error : "",
    ),
  );

/** The idle-minutes choices for the Settings screen: the usual ones plus the saved value when it is none of them, in order. */
export const IDLE_CHOICES = [5, 10, 15, 30, 60, 120, 240];
export const idleChoices = (saved: number): number[] => [...new Set([...IDLE_CHOICES, saved])].sort((a, b) => a - b);
