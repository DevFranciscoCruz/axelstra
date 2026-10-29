import { fmtTokens } from "./usageFormat";
import { P } from "./strings.panels";
import { coded } from "./msg";
import { stampIso, stampLocal } from "./time";

export type TimelineEvent = {
  at: number;
  kind: "commit" | "merge" | "integrated" | "report";
  title: string;
  detail: string;
  /** set when the daemon wrote the title (not a commit subject): read it in the UI language */
  title_code?: string;
  title_params?: Record<string, string>;
};
export type TimelineTokens = { input: number; output: number; cache_read: number; cache_write: number };
export type TimelineCheck = { name: string; state: string; link: string };
export type Timeline = {
  branch: string;
  base: string;
  /** newest first */
  events: TimelineEvent[];
  tokens: TimelineTokens | null;
  ci: { number: number; title: string; url: string; ci: string; checks: TimelineCheck[] } | null;
};

const T = P.timeline;

export const eventTitle = (e: TimelineEvent): string => coded(e.title_code, e.title_params, e.title);

/** the panel writes dates in the UI language; the exported markdown keeps ISO (`stampIso`) so it reads the same everywhere */
export const stamp = stampLocal;

export const tokenLine = (t: TimelineTokens): string =>
  T.tokens(fmtTokens(t.input + t.output), fmtTokens(t.input), fmtTokens(t.output), fmtTokens(t.cache_read));

export const ciLine = (ci: NonNullable<Timeline["ci"]>): string => {
  const failed = ci.checks.filter((c) => c.state === "fail").map((c) => c.name);
  return `#${ci.number} ${T.ci[ci.ci] ?? ci.ci}${failed.length ? ` (${failed.join(", ")})` : ""}`;
};

/** Escapes what would break a markdown list line. */
const SPECIAL = new Set(["\\", "`", "*", "_", "[", "]", "<", ">"]);
const md = (s: string) => [...s.replace(/[\r\n]+/g, " ")].map((c) => (SPECIAL.has(c) ? "\\" + c : c)).join("");

/** The timeline as a markdown document, oldest first, so it reads like a log. */
export function toMarkdown(t: Timeline, worktree: string): string {
  const out = [`# ${T.exportTitle(md(worktree))}`, "", `- ${T.branch}: \`${t.branch}\` (${T.base} \`${t.base}\`)`];
  if (t.tokens) out.push(`- ${T.tokensLabel}: ${tokenLine(t.tokens)}`);
  if (t.ci) out.push(`- ${T.ciLabel}: ${md(ciLine(t.ci))}`);
  out.push("", `## ${T.events}`, "");
  if (!t.events.length) out.push(`_${T.empty}_`);
  for (const e of [...t.events].reverse()) {
    out.push(
      `- **${stampIso(e.at)}** · ${T.kinds[e.kind] ?? e.kind}: ${md(eventTitle(e))}${e.detail ? ` — ${md(e.detail)}` : ""}`,
    );
  }
  return out.join("\n") + "\n";
}
