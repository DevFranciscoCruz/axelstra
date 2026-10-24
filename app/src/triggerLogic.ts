import type { TriggerEvent, TriggerFiring, TriggerLimits, TriggerPreview, TriggerState } from "./api";
import { ago } from "./time";
import { P } from "./strings.panels";

/** The first enabling is confirmed by the user typing a word; later toggles are not. */
export const needsConfirm = (s: Pick<TriggerState, "confirmed_at">): boolean => s.confirmed_at == null;

/** `ax/fix-login` -> `fix-login`; the last path segment of anything else. */
export const branchLabel = (branch: string): string => branch.replace(/^ax\//, "") || branch;

/** The dialog text for a first enabling: what will run, where, how often and where it is logged. */
export const confirmMessage = (pv: TriggerPreview): string =>
  P.triggers.confirmMessage({
    agent: pv.agent,
    prompt: pv.prompt,
    cooldownMin: Math.round(pv.limits.cooldown_secs / 60),
    perHour: pv.limits.max_per_hour,
    pollSecs: pv.limits.poll_secs,
    auditMethod: pv.audit_method,
  });

/** The last path segment of a worktree path, for the list of firings. */
const folder = (p: string): string =>
  p
    .replace(/[\\/]+$/, "")
    .split(/[\\/]/)
    .pop() ?? p;

export const firingText = (f: TriggerFiring, nowSec = Date.now() / 1000): string =>
  P.triggers.firing(ago(nowSec - f.ts), folder(f.worktree), f.checks.slice(0, 3).join(", "));

export const firingKind = (f: TriggerFiring): string =>
  f.kind === "fired" ? P.triggers.kindFired : f.kind === "limited" ? P.triggers.kindLimited : P.triggers.kindFailed;

/** Toast text for something the trigger did. */
export const eventText = (e: TriggerEvent): string => {
  const b = branchLabel(e.branch);
  if (e.kind === "ci_fix") return P.triggers.firedToast(b);
  if (e.kind === "ci_fix_limited") return P.triggers.limitedToast(b, e.detail ?? "");
  return P.triggers.failedToast(b, e.detail ?? "");
};

const clampInt = (n: number, [lo, hi]: [number, number]): number => Math.min(hi, Math.max(lo, Math.round(n)));

/** Minutes typed by the user -> a patch of the cooldown in seconds within the daemon's range; null when it is not a number. */
export const cooldownPatch = (minutes: string, l: TriggerLimits): number | null => {
  const n = Number(minutes.replace(",", "."));
  return minutes.trim() === "" || !Number.isFinite(n) ? null : clampInt(n * 60, l.cooldown_range);
};

export const perHourPatch = (text: string, l: TriggerLimits): number | null => {
  const n = Number(text);
  return text.trim() === "" || !Number.isFinite(n) ? null : clampInt(n, l.max_per_hour_range);
};

export type ToggleDeps = {
  preview: (project: string) => Promise<TriggerPreview | null>;
  ask: (
    message: string,
    opts: { title: string; kind: "warning"; okLabel: string; typeToConfirm: string },
  ) => Promise<boolean>;
  set: (project: string, patch: { ci_fix: boolean; confirm?: string }) => Promise<TriggerState>;
};

/** Turns the trigger on or off. Off needs nothing; the first "on" shows what will happen and needs the word typed; later "on"s go straight through. Returns the new state, or null when nothing changed. */
export const toggleTrigger = async (
  project: string,
  on: boolean,
  s: Pick<TriggerState, "confirmed_at">,
  d: ToggleDeps,
): Promise<TriggerState | null> => {
  if (!on) return d.set(project, { ci_fix: false });
  if (!needsConfirm(s)) return d.set(project, { ci_fix: true });
  const pv = await d.preview(project);
  if (!pv) return null;
  const ok = await d.ask(confirmMessage(pv), {
    title: P.triggers.confirmTitle,
    kind: "warning",
    okLabel: P.triggers.confirmOk,
    typeToConfirm: pv.confirm_word,
  });
  return ok ? d.set(project, { ci_fix: true, confirm: pv.confirm_word }) : null;
};
