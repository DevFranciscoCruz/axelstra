import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { createLive } from "./live";
import { TokenUsage, agentUsage, worktreeUsage } from "./api";
import { activeSession, projects, sessions } from "./store";
import { WtUsage, fmtTokens as fmt, ranked, spentOf, totalOf, usageTip } from "./usageFormat";
import { Button, cn } from "./ui";
import { P } from "./strings.panels";
import { B } from "./strings.budget";
import { BudgetMeter } from "./BudgetMeter";
import { budgetRows, loadBudgets } from "./budgetStore";
import { rowsFor, worstLevel } from "./budgetLogic";
import { fmtNumber } from "./i18n";

const [wtUsage, setWtUsage] = createSignal<WtUsage[]>([]);

/** per-worktree totals are shared by this view and the status bar; `createLive` runs one load at a time */
export const loadWtUsage = () => {
  if (document.hidden) return;
  const paths = projects().flatMap((p) => p.worktrees.map((w) => w.path));
  if (!paths.length) return;
  void loadBudgets(paths);
  void worktreeUsage(paths)
    .then(setWtUsage)
    .catch(() => {
      /* an older daemon does not know the call; the totals stay empty */
    });
};

/** keeps the totals fresh (daemon events, no timer) while the component that calls this is on screen */
export const pollWtUsage = () => {
  onMount(() => {
    const t = createLive(loadWtUsage, ["usage.changed", "budget.events", "session.changed"]);
    t.now();
    onCleanup(t.stop);
  });
  return wtUsage;
};

/** what the sidebar calls a worktree: its branch, else the folder name */
export const wtName = (path: string) => {
  const w = projects().flatMap((p) => p.worktrees).find((x) => x.path === path);
  return w?.branch ?? path.split(/[\\/]/).filter(Boolean).pop() ?? path;
};

export const wtUsageTip = (list: WtUsage[]) => usageTip(list, wtName);

/** tokens spent per worktree (Claude and Codex), as a small list opened from the chip */
function WorktreeUsageList() {
  const list = pollWtUsage();
  const [open, setOpen] = createSignal(false);
  const rows = createMemo(() => ranked(list()));
  const total = createMemo(() => totalOf(rows()));
  // the worst budget level anywhere, so the chip shows it without opening the list
  const level = createMemo(() => worstLevel(budgetRows()));
  const chipNote = () => (level() === "over" ? B.chipOver : level() === "warn" ? B.chipWarn : "");
  const close = (e: Event) => {
    if (!(e.target as HTMLElement | null)?.closest?.(".wt-usage")) setOpen(false);
  };
  let btn: HTMLButtonElement | undefined;
  let pop: HTMLDivElement | undefined;
  const esc = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || !open()) return;
    setOpen(false);
    btn?.focus();
  };
  // focus moves into the popover when it opens and goes back to the chip when it closes
  createEffect(() => {
    if (open()) queueMicrotask(() => pop?.focus());
  });
  document.addEventListener("mousedown", close);
  document.addEventListener("keydown", esc);
  onCleanup(() => {
    document.removeEventListener("mousedown", close);
    document.removeEventListener("keydown", esc);
  });
  return (
    <Show when={rows().length}>
      <span class="wt-usage self-center">
        <Button ref={btn} variant="link" size="bare" class={cn("cursor-default self-center px-2 text-xs hover:text-muted-foreground", level() === "over" && "text-[var(--danger)]", level() === "warn" && "text-[var(--warn)]")} title={[wtUsageTip(list()), chipNote()].filter(Boolean).join("\n")} aria-label={[P.usage.perWtTotal(fmt(spentOf(total()))), chipNote()].filter(Boolean).join(". ")} aria-haspopup="dialog" aria-expanded={open()} onClick={() => setOpen(!open())}>
          Σ {fmt(spentOf(total()))}
        </Button>
        <Show when={open()}>
          <div ref={pop} class="wt-usage-pop" role="dialog" aria-label={P.usage.perWt} tabindex="-1">
            <div class="wup-note">{P.usage.note}</div>
            <For each={rows()}>
              {(u) => (
                <>
                <div class="wup-row" title={P.usage.rowTip(u.worktree, fmtNumber(u.input), fmtNumber(u.output), fmtNumber(u.cache_write), fmtNumber(u.cache_read), u.claude_sessions, u.codex_sessions)}>
                  <span class="wup-name">{wtName(u.worktree)}</span>
                  <span>{fmt(spentOf(u))}</span>
                </div>
                <Show when={rowsFor(budgetRows(), u.worktree).length}>
                  <div class="wup-bud">
                    <For each={rowsFor(budgetRows(), u.worktree)}>{(r) => <BudgetMeter row={r} name={wtName(u.worktree)} />}</For>
                  </div>
                </Show>
                </>
              )}
            </For>
            <div class="wup-row wup-total">
              <span>{P.usage.total}</span>
              <span>{fmt(spentOf(total()))}</span>
            </div>
            <div class="wup-note wup-foot">{P.usage.foot(fmt(total().cache_read))}</div>
          </div>
        </Show>
      </span>
    </Show>
  );
}

/** tokens spent by the Claude conversation in the active tab, read from Claude's own transcript */
export function UsageChip() {
  const [usage, setUsage] = createSignal<TokenUsage | null>(null);
  // only a change of tab or conversation restarts the polling, not every refresh of the session list
  const target = createMemo(() => {
    const s = sessions().find((x) => x.id === activeSession());
    const cwd = s?.meta?.worktree;
    if (!s || s.meta?.kind !== "agent" || s.meta.agent !== "claude" || !cwd) return "";
    return JSON.stringify([cwd, s.meta?.resume ?? s.meta?.key ?? ""]);
  });
  createEffect(on(target, (tgt) => {
    const [cwd, conv] = tgt ? (JSON.parse(tgt) as [string, string]) : ["", ""];
    setUsage(null);
    if (!cwd) return;
    let alive = true;
    const load = () =>
      agentUsage(cwd, conv)
        .then((u) => alive && setUsage(u))
        .catch(() => alive && setUsage(null));
    const t = createLive(load, ["usage.changed", "session.changed"]);
    if (!document.hidden) t.now();
    onCleanup(() => {
      alive = false;
      t.stop();
    });
  }));
  const spent = () => (usage() ? usage()!.input + usage()!.output + usage()!.cache_write : 0);
  const ctx = () => usage()?.context ?? 0;
  return (
    <>
    <WorktreeUsageList />
    <Show when={usage() && usage()!.messages > 0}>
      <span
        class="usage-chip"
        title={P.usage.chipTip(fmtNumber(ctx()), fmt(spent()), fmtNumber(usage()!.input), fmtNumber(usage()!.output), fmtNumber(usage()!.cache_write), fmtNumber(usage()!.cache_read), usage()!.messages)}
      >
        {fmt(ctx())} ctx
      </span>
    </Show>
    </>
  );
}
