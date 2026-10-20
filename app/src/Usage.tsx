import { Show, createEffect, createSignal, onCleanup } from "solid-js";
import { TokenUsage, agentUsage } from "./api";
import { activeSession, sessions } from "./store";

const fmt = (n: number) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : String(n));

/** tokens spent by the Claude conversation in the active tab, read from Claude's own transcript */
export function UsageChip() {
  const [usage, setUsage] = createSignal<TokenUsage | null>(null);
  createEffect(() => {
    const s = sessions().find((x) => x.id === activeSession());
    const cwd = s?.meta?.worktree;
    const conv = s?.meta?.resume ?? s?.meta?.key;
    setUsage(null);
    if (!s || s.meta?.kind !== "agent" || s.meta.agent !== "claude" || !cwd) return;
    let alive = true;
    const load = () => {
      if (document.hidden) return;
      void agentUsage(cwd, conv)
        .then((u) => alive && setUsage(u))
        .catch(() => alive && setUsage(null));
    };
    load();
    const t = setInterval(load, 15000);
    onCleanup(() => {
      alive = false;
      clearInterval(t);
    });
  });
  const spent = () => (usage() ? usage()!.input + usage()!.output + usage()!.cache_write : 0);
  return (
    <Show when={usage() && usage()!.messages > 0}>
      <span
        class="usage-chip"
        title={`Entrada ${usage()!.input.toLocaleString()} · saída ${usage()!.output.toLocaleString()} · cache escrita ${usage()!.cache_write.toLocaleString()} · cache lida ${usage()!.cache_read.toLocaleString()} (${usage()!.messages} mensagens)`}
      >
        {fmt(spent())} tok
      </span>
    </Show>
  );
}
