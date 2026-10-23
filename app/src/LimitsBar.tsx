import { Show, createEffect, createMemo, createSignal, onCleanup } from "solid-js";
import { ClaudeStatus, CodexLimits, CodexWindow, agentStatus, codexLimits } from "./api";
import { activeSession, sessions } from "./store";

/** time until a reset: "1h 49m", "4d", "29d 23h" */
export const until = (secs: number) => {
  const m = Math.max(0, Math.round(secs / 60));
  const d = Math.floor(m / 1440);
  const h = Math.floor((m % 1440) / 60);
  if (d >= 1) return h && d < 10 ? `${d}d ${h}h` : `${d}d`;
  return h ? `${h}h ${m % 60}m` : `${m}m`;
};

const kfmt = (n: number) => (n >= 1e6 ? `${+(n / 1e6).toFixed(1)}M` : `${Math.round(n / 1e3)}k`);

const svg = { width: 14, height: 14, viewBox: "0 0 16 16", fill: "currentColor" } as const;
const ClaudeIcon = () => (
  <svg {...svg} style={{ color: "#d97757" }}>
    <path d="M8 1l1.2 4.3L13.5 3l-2.3 4.2L15 8l-3.8.8 2.3 4.2-4.3-2.3L8 15l-1.2-4.3L2.5 13l2.3-4.2L1 8l3.8-.8L2.5 3l4.3 2.3z" />
  </svg>
);
const CodexIcon = () => (
  <svg {...svg} fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round">
    <path d="M8 1.5l5.5 3.2v6.6L8 14.5l-5.5-3.2V4.7zM8 1.5v13M2.5 4.7l11 6.6M13.5 4.7l-11 6.6" />
  </svg>
);

const Meter = (p: { pct?: number; label: string; tip: string }) => (
  <Show when={p.pct !== undefined}>
    <span class="lim" title={p.tip}>
      <span class="lim-bar">
        <i classList={{ warn: p.pct! >= 70, bad: p.pct! >= 90 }} style={{ width: `${Math.min(100, p.pct!)}%` }} />
      </span>
      <span>{Math.round(p.pct!)}% usado</span>
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
  createEffect(() => {
    const key = claudeKey();
    let alive = true;
    const load = () => {
      if (document.hidden) return;
      setNow(Date.now() / 1000);
      if (key) void agentStatus(key).then((v) => alive && v && setClaude(v)).catch(() => 0);
    };
    load();
    const t = setInterval(load, 5000);
    onCleanup(() => {
      alive = false;
      clearInterval(t);
    });
  });
  createEffect(() => {
    const load = () => {
      if (!document.hidden) void codexLimits().then((v) => v && setCodex(v)).catch(() => 0);
    };
    load();
    const t = setInterval(load, 30000);
    onCleanup(() => clearInterval(t));
  });

  const five = () => claude()?.rate_limits?.five_hour;
  const week = () => claude()?.rate_limits?.seven_day;
  const cw = () => claude()?.context_window;
  const left = (r?: { resets_at?: number }) => (r?.resets_at ? until(r.resets_at - now()) : "");
  const ctxTotal = () => cw()?.context_window_size ?? 0;
  const ctxPct = () => cw()?.used_percentage;
  const cx = () => codex();

  return (
    <div class="limits">
      <Show when={ctxPct() !== undefined}>
        <span class="lim" title={`Contexto da conversa: ${Math.round(ctxPct()!)}% de ${kfmt(ctxTotal())}`}>
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
        <Meter pct={five()?.used_percentage} label={left(five())} tip="Claude: limite de 5 horas" />
        <Meter pct={week()?.used_percentage} label={left(week())} tip="Claude: limite semanal" />
      </Show>
      <Show when={cx()?.primary || cx()?.secondary}>
        <CodexIcon />
        <Meter pct={cx()?.primary?.used_percent} label={fmtSecs(codexSecs(cx()?.primary, cx()?.as_of))} tip="Codex: janela curta" />
        <Meter pct={cx()?.secondary?.used_percent} label={fmtSecs(codexSecs(cx()?.secondary, cx()?.as_of))} tip="Codex: janela semanal" />
      </Show>
    </div>
  );
}

const fmtSecs = (s?: number) => (s === undefined ? "" : until(s));
