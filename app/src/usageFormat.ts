/** tokens per worktree, summed over the Claude and Codex conversations that ran there */
export type WtUsage = { worktree: string; input: number; output: number; cache_read: number; cache_write: number; claude_sessions: number; codex_sessions: number };

export const fmtTokens = (n: number) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : String(n));

/** what was paid for: input, output and cache writes (cache reads are shown apart, they cost a fraction) */
export const spentOf = (u: Pick<WtUsage, "input" | "output" | "cache_write">) => u.input + u.output + u.cache_write;

/** worktrees that used an agent at all, the heaviest first */
export const ranked = (list: WtUsage[]) => list.filter((u) => spentOf(u) + u.cache_read > 0).sort((a, b) => spentOf(b) - spentOf(a));

export const totalOf = (list: WtUsage[]): Omit<WtUsage, "worktree"> =>
  list.reduce(
    (t, u) => ({ input: t.input + u.input, output: t.output + u.output, cache_read: t.cache_read + u.cache_read, cache_write: t.cache_write + u.cache_write, claude_sessions: t.claude_sessions + u.claude_sessions, codex_sessions: t.codex_sessions + u.codex_sessions }),
    { input: 0, output: 0, cache_read: 0, cache_write: 0, claude_sessions: 0, codex_sessions: 0 },
  );

/** one line per worktree for a tooltip; `nameOf` turns a path into what the sidebar calls it */
export const usageTip = (list: WtUsage[], nameOf: (path: string) => string) => {
  const rows = ranked(list);
  if (!rows.length) return "Nenhum agente usou tokens nestes worktrees";
  const t = totalOf(rows);
  const head = `Tokens dos agentes (Claude e Codex): ${fmtTokens(spentOf(t))} gastos · ${fmtTokens(t.cache_read)} de cache lida`;
  return [head, ...rows.map((u) => `${nameOf(u.worktree)}: ${fmtTokens(spentOf(u))} (entrada ${fmtTokens(u.input)} · saída ${fmtTokens(u.output)} · cache ${fmtTokens(u.cache_write)}/${fmtTokens(u.cache_read)})`)].join("\n");
};
