import { P } from "./strings.panels";
import type { KCard, OrchTrust, OrchTrustCard } from "./api";

/** `bypass*` modes (bypassPermissions, bypass-permissions…) run an agent with no supervision at all */
export const isBypass = (mode: string | null | undefined) => /^bypass/i.test((mode ?? "").trim());

/** cards the user must opt into explicitly: the daemon's own flag, or a bypass-looking mode */
export const bypassCards = (cards: OrchTrustCard[]) => cards.filter((c) => c.bypass || isBypass(c.permission_mode));

/** one line per card for the confirm text; bypass cards are marked so they stand out */
export const trustLines = (cards: OrchTrustCard[]) =>
  cards.map((c) => {
    const flag = c.bypass || isBypass(c.permission_mode) ? "⚠ " : "";
    return `${flag}${c.id}: ${c.agent} (${c.permission_mode || P.kanban.defaultMode})${c.acceptance ? P.kanban.acceptance(c.acceptance) : ""}`;
  });

export type StartPlan = {
  /** the confirm body */
  message: string;
  /** acceptance commands run in a shell, or a bypass card needs an explicit yes */
  warn: boolean;
  bypass: string[];
  trustAcceptance: boolean;
  /** hash of the cards the user saw: the daemon refuses if they change meanwhile */
  pin?: string;
};

/** What the user is asked and what is sent on yes. `trust` is null when the daemon has no `orch.trust_info`. */
export function planStart(n: number, trust: OrchTrust | null, legacyCmds: string[]): StartPlan {
  const head = P.kanban.planHead(n);
  if (!trust) {
    return {
      message: head + (legacyCmds.length ? P.kanban.legacyCmds(legacyCmds.join("\n")) : ""),
      warn: legacyCmds.length > 0,
      bypass: [],
      trustAcceptance: legacyCmds.length > 0,
    };
  }
  const bypass = bypassCards(trust.cards).map((c) => c.id);
  const cmds = trust.cards.filter((c) => c.acceptance).length;
  const bypassNote = bypass.length ? P.kanban.bypassNote(bypass.length, bypass.join(", ")) : "";
  return {
    message: `${head}\n\n${trustLines(trust.cards).join("\n")}${bypassNote}${cmds ? P.kanban.trustCmds : ""}`,
    warn: cmds > 0 || bypass.length > 0,
    bypass,
    trustAcceptance: cmds > 0,
    pin: trust.pin,
  };
}

/** the payload that re-creates a deleted card (same id, so dependents still point at it) */
export function restorePayload(c: KCard): Record<string, unknown> {
  return {
    id: c.meta.id,
    title: c.meta.title,
    body: c.body,
    status: c.meta.status,
    agent: c.meta.agent,
    priority: c.meta.priority,
    depends_on: c.meta.depends_on,
    ...(c.meta.acceptance ? { acceptance: c.meta.acceptance } : {}),
  };
}
