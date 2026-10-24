import { fmtTokens } from "./usageFormat";
import { P } from "./strings.panels";

export type TimelineEvent = {
  at: number;
  kind: "commit" | "merge" | "integrated" | "report";
  title: string;
  detail: string;
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

const pad = (n: number) => String(n).padStart(2, "0");
/** "2026-10-24 13:05" in local time: one wording for the panel and the export */
export const stamp = (at: number): string => {
  const d = new Date(at * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
};

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
      `- **${stamp(e.at)}** · ${T.kinds[e.kind] ?? e.kind}: ${md(e.title)}${e.detail ? ` — ${md(e.detail)}` : ""}`,
    );
  }
  return out.join("\n") + "\n";
}
