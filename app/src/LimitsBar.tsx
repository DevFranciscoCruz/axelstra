import { For, Show, createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { Icon } from "./Icon";
import { createLive } from "./live";
import { ClaudeStatus, CodexLimits, CodexWindow, agentStatus, codexLimits } from "./api";
import { activeSession, sessions } from "./store";
import { P } from "./strings.panels";
import { BudgetMeter } from "./BudgetMeter";
import { budgetRows } from "./budgetStore";
import { rowsFor } from "./budgetLogic";
import { wtName } from "./Usage";
import { fmtNumber } from "./i18n";

/** time until a reset: "1h 49m", "4d", "29d 23h" */
export const until = (secs: number) => {
  const m = Math.max(0, Math.round(secs / 60));
  const d = Math.floor(m / 1440);
  const h = Math.floor((m % 1440) / 60);
  if (d >= 1) return h && d < 10 ? `${d}d ${h}h` : `${d}d`;
  return h ? `${h}h ${m % 60}m` : `${m}m`;
};

const kfmt = (n: number) => (n >= 1e6 ? `${fmtNumber(n / 1e6, { maximumFractionDigits: 1 })}M` : `${Math.round(n / 1e3)}k`);

const ClaudeIcon = () => <Icon name="claudeMark" size={14} class="claude-ico" />;
const CodexIcon = () => <Icon name="codexMark" size={14} />;

const Meter = (p: { pct?: number; label: string; tip: string }) => (
  <Show when={p.pct !== undefined}>
    <span class="lim" title={p.tip}>
      <span class="lim-bar">
        <i classList={{ warn: p.pct! >= 70, bad: p.pct! >= 90 }} style={{ width: `${Math.min(100, p.pct!)}%` }} />
      </span>
      <span>{P.limits.used(Math.round(p.pct!))}</span>
      <Show when={p.label}>
        <span class="lim-dim">{p.label}</span>
      </Show>
    </span>
  </Show>
);

// the limits belong to the account, so the last values stay on screen when a shell tab is active
const [claude, setClaude] = createSignal<ClaudeStatus | null>(null);
const [codex, setCodex] = createSignal<CodexLimits | null>(null);
const [now, setNow] = createSignal(Date.now() / 1000);

const codexSecs = (w?: CodexWindow, asOf = 0) => {
  if (!w) return undefined;
  if (w.resets_at) return w.resets_at - now();
  if (w.resets_in_seconds !== undefined) return w.resets_in_seconds - (now() - asOf);
  return undefined;
};

/** Claude's 5 h / weekly limits and context (from its status line) and Codex's limits, next to the status bar. */
export function LimitsBar() {
  const claudeKey = createMemo(() => {
    const s = sessions().find((x) => x.id === activeSession());
    return s?.meta?.agent === "claude" ? s.meta.key : undefined;
  });
  createEffect(on(claudeKey, (key) => {
    let alive = true;
    const load = () => {
      setNow(Date.now() / 1000);
      return key ? agentStatus(key).then((v) => alive && v && setClaude(v)).catch(() => 0) : undefined;
    };
    // the status line file changes when the agent works (`limits.changed`); a new key restarts this effect, the window showing again also looks
    const t = createLive(load, ["limits.changed"], { debounceMs: 300 });
    t.now();
    onCleanup(() => {
      alive = false;
      t.stop();
    });
  }));
  createEffect(() => {
    const load = () => codexLimits().then((v) => v && setCodex(v)).catch(() => 0);
    const t = createLive(load, ["limits.changed"], { debounceMs: 300 });
    t.now();
    onCleanup(t.stop);
  });

  const five = () => claude()?.rate_limits?.five_hour;
  const week = () => claude()?.rate_limits?.seven_day;
  const cw = () => claude()?.context_window;
  const left = (r?: { resets_at?: number }) => (r?.resets_at ? until(r.resets_at - now()) : "");
  const ctxTotal = () => cw()?.context_window_size ?? 0;
  const ctxPct = () => cw()?.used_percentage;
  const cx = () => codex();
  // the token budgets of the worktree the active tab belongs to
  const wt = () => sessions().find((x) => x.id === activeSession())?.meta?.worktree;
  const budgets = () => (wt() ? rowsFor(budgetRows(), wt()!) : []);

  return (
    <div class="limits">
      <Show when={ctxPct() !== undefined}>
        <span class="lim" title={P.limits.ctxTip(Math.round(ctxPct()!), kfmt(ctxTotal()))}>
          <span class="lim-dim">ctx</span>
          <span class="lim-bar">
            <i classList={{ warn: ctxPct()! >= 70, bad: ctxPct()! >= 90 }} style={{ width: `${Math.min(100, ctxPct()!)}%` }} />
          </span>
          <span>
            {kfmt((ctxTotal() * ctxPct()!) / 100)}/{kfmt(ctxTotal())}
          </span>
        </span>
      </Show>
      <Show when={five() || week()}>
        <ClaudeIcon />
        <Meter pct={five()?.used_percentage} label={left(five())} tip={P.limits.claude5h} />
        <Meter pct={week()?.used_percentage} label={left(week())} tip={P.limits.claudeWeek} />
      </Show>
      <Show when={cx()?.primary || cx()?.secondary}>
        <CodexIcon />
        <Meter pct={cx()?.primary?.used_percent} label={fmtSecs(codexSecs(cx()?.primary, cx()?.as_of))} tip={P.limits.codexShort} />
        <Meter pct={cx()?.secondary?.used_percent} label={fmtSecs(codexSecs(cx()?.secondary, cx()?.as_of))} tip={P.limits.codexWeek} />
      </Show>
      <For each={budgets()}>{(r) => <BudgetMeter row={r} name={wtName(wt()!)} />}</For>
    </div>
  );
}

const fmtSecs = (s?: number) => (s === undefined ? "" : until(s));
