import { P } from "./strings.panels";

export type WtRisk = { dirty?: number; ahead: number; upstream?: boolean | string | null } | undefined;

/** what removing a worktree would lose, in PT-PT phrases */
export function removeRisks(st: WtRisk, live: number): string[] {
  const risks: string[] = [];
  if (st?.dirty) risks.push(P.risk.dirty(st.dirty));
  if (st && st.ahead > 0) risks.push(st.upstream ? P.risk.unpushed(st.ahead) : P.risk.localOnly(st.ahead));
  if (live) risks.push(P.risk.live(live));
  return risks;
}

/** work that exists nowhere else (uncommitted or unpushed) makes the user type the name; running terminals alone do not */
export const needsTypedName = (st: WtRisk) => !!st && ((st.dirty ?? 0) > 0 || st.ahead > 0);
