export type WtRisk = { dirty?: number; ahead: number; upstream?: boolean | string | null } | undefined;

/** what removing a worktree would lose, in PT-PT phrases */
export function removeRisks(st: WtRisk, live: number): string[] {
  const risks: string[] = [];
  if (st?.dirty) risks.push(`${st.dirty} ${st.dirty === 1 ? "ficheiro com alterações" : "ficheiros com alterações"} por guardar`);
  if (st && st.ahead > 0) risks.push(st.upstream ? `${st.ahead} ${st.ahead === 1 ? "commit" : "commits"} por enviar` : `${st.ahead} ${st.ahead === 1 ? "commit" : "commits"} que só existem aqui (sem remoto)`);
  if (live) risks.push(`${live} ${live === 1 ? "terminal ou agente a correr" : "terminais ou agentes a correr"}`);
  return risks;
}

/** work that exists nowhere else (uncommitted or unpushed) makes the user type the name; running terminals alone do not */
export const needsTypedName = (st: WtRisk) => !!st && ((st.dirty ?? 0) > 0 || st.ahead > 0);
