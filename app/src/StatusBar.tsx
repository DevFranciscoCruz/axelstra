import { For, Show, createEffect, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { createPersistedFlag } from "./persist";
import { createLive } from "./live";
import { S } from "./strings";
import { invoke } from "@tauri-apps/api/core";
import { findWorktree, openSession, overview, refreshOverview, refreshProjects, selectWorktree, selectedWorktree, sessionState, setShowShortcuts, sessions, showNotice, tabLabel, watchOverview } from "./store";
import { worktreeSweep } from "./api";
import { errorText } from "./appError";
import { ask } from "./Confirm";
import { StateDot } from "./StateDot";
import { Icon } from "./Icon";
import { countsFor, groupAgents, mergedList, type AgentEntry } from "./statusGroups";
import { LimitsBar } from "./LimitsBar";
import { VersionChip } from "./About";
import { hoverIntent, keepInside } from "./hoverIntent";
import { NotificationBell } from "./NotificationBell";
import { pollWtUsage, wtUsageTip } from "./Usage";
import { ranked, spentOf, totalOf, fmtTokens } from "./usageFormat";
import { Button, cn } from "./ui";
import { K } from "./keyboard";
import "./shell.css";
import { fmtFixed, fmtNumber } from "./i18n";

type Usage = { app: number; daemon: number; sessions: number; ports: { port: number; pid: number }[]; cpu: number | null; gpu: number | null; gpu_adapters: [string, number][] };

const KEY = "axelstra.keepAwake";

const fmt = (b: number) => (b >= 1024 ** 3 ? `${fmtFixed(b / 1024 ** 3, 2)} GB` : `${Math.round(b / 1024 ** 2)} MB`);

const svg = { width: 14, height: 14, viewBox: "0 0 16 16", fill: "none", stroke: "currentColor", "stroke-width": 1.3, "stroke-linecap": "round", "stroke-linejoin": "round" } as const;

let popSeq = 0;

/** A status bar chip that opens a popover: a button (keyboard and screen readers), also opened by hovering or keyboard focus; closed by Escape, a click outside or leaving it. */
function Popover(props: { chip: JSX.Element; label: string; onOpen?: () => void; children: (close: () => void) => JSX.Element }) {
  const [open, setOpen] = createSignal(false);
  const [shift, setShift] = createSignal(0);
  const id = `sb-pop-${++popSeq}`;
  let pinned = false; // opened by a click: hovering away does not close it
  let box!: HTMLSpanElement;
  let pop: HTMLDivElement | undefined;
  const hover = hoverIntent((v) => (v ? setOpen(true) : !pinned && setOpen(false)));
  const close = () => {
    hover.cancel();
    pinned = false;
    setOpen(false);
  };
  onCleanup(hover.cancel);
  createEffect(() => {
    if (!open()) {
      setShift(0);
      return;
    }
    props.onOpen?.();
    requestAnimationFrame(() => pop && setShift(keepInside(pop.getBoundingClientRect().right - shift(), window.innerWidth)));
    const away = (e: Event) => !box.contains(e.target as Node) && close();
    document.addEventListener("pointerdown", away);
    onCleanup(() => document.removeEventListener("pointerdown", away));
  });
  return (
    <span
      class="sb-pop-wrap"
      ref={box}
      onPointerEnter={(e) => e.pointerType !== "touch" && hover.enter()}
      onPointerLeave={(e) => e.pointerType !== "touch" && hover.leave()}
      onKeyDown={(e) => {
        if (e.key === "Escape" && open()) {
          e.stopPropagation();
          close();
          box.querySelector<HTMLElement>(".sb-item")?.focus();
        }
      }}
      onFocusIn={(e) => e.target instanceof HTMLElement && e.target.matches(":focus-visible") && setOpen(true)}
      onFocusOut={(e) => !box.contains(e.relatedTarget as Node | null) && close()}
    >
      <Button
        variant="row"
        size="xs"
        class="sb-item cursor-pointer hover:bg-foreground/10 hover:text-foreground"
        aria-haspopup="true"
        aria-expanded={open()}
        aria-describedby={open() ? id : undefined}
        aria-label={props.label}
        onClick={() => {
          hover.cancel();
          if (open() && pinned) return close();
          pinned = true;
          setOpen(true);
        }}
      >
        {props.chip}
      </Button>
      <Show when={open()}>
        <div class="sb-pop" id={id} ref={pop} role="group" aria-label={props.label} style={shift() ? { left: `${shift()}px` } : undefined}>
          {props.children(close)}
        </div>
      </Show>
    </span>
  );
}

/** Thin bar under the whole window: keep-awake, memory used by Axelstra, running terminals, dev-server ports. */
export function StatusBar() {
  const [awake, setAwake] = createPersistedFlag(KEY, false);
  const [usage, setUsage] = createSignal<Usage | null>(null);
  const groups = () => groupAgents(sessions(), sessionState, tabLabel);
  const running = () => groups().running.length;
  const currentProject = () => findWorktree(selectedWorktree())?.project.id ?? null;
  const counts = () => countsFor(overview(), currentProject());
  const loadOverview = () => void refreshOverview();
  const clean = async () => {
    const list = mergedList(overview());
    if (!list.length) return;
    const ok = await ask(S.status.cleanAsk(list.length, list.map((m) => `${m.project}: ${m.branch}`).join("\n")), { title: S.status.cleanTitle, kind: "warning", okLabel: S.status.remove, cancelLabel: S.cancel });
    if (!ok) return;
    try {
      const done = (await worktreeSweep()).filter((x) => x.removed).length;
      showNotice(S.status.removed(done));
    } catch (e) {
      showNotice(errorText(e));
    }
    await refreshProjects();
    loadOverview();
  };
  const tokens = pollWtUsage();
  const [awakeOk, setAwakeOk] = createSignal(true);

  const toggle = () => {
    const on = !awake();
    setAwake(on);
    void invoke<boolean>("keep_awake", { on })
      .then((held) => setAwakeOk(held || !on))
      .catch(() => 0);
  };

  const poll = async () => {
    if (document.hidden) return;
    try {
      setUsage(await invoke<Usage>("mem_usage"));
    } catch {
      /* transient */
    }
  };

  onMount(() => {
    if (awake()) void invoke<boolean>("keep_awake", { on: true }).then(setAwakeOk).catch(() => 0);
    // memory is read when sessions change and when the window shows again; the counts come from the events the sidebar shares (store.watchOverview)
    const mem = createLive(poll, ["session.changed", "resources.changed"]);
    mem.now();
    const unwatch = watchOverview();
    onCleanup(() => {
      mem.stop();
      unwatch();
    });
  });

  const total = () => {
    const u = usage();
    return u ? u.app + u.daemon + u.sessions : 0;
  };
  const memTip = () => {
    const u = usage();
    if (!u) return S.status.memTitle;
    return S.status.memTip(fmt(u.app), fmt(u.daemon), fmt(u.sessions));
  };
  const ports = () => usage()?.ports ?? [];
  const pct = (n: number) => (n < 10 ? fmtFixed(n, 1) : fmtNumber(Math.round(n)));

  return (
    <div class="statusbar">
      <Button variant="row" size="xs" class={cn("sb-item cursor-pointer hover:bg-foreground/10 hover:text-foreground", awake() && "text-[var(--ok)]")} title={awakeOk() ? S.status.awake : S.status.awakeNA} aria-pressed={awake()} onClick={toggle}>
        <Icon name="coffee" size={14} />
        <span class="size-2 rounded-full" classList={{ "bg-[var(--ok)]": awake() && awakeOk(), "bg-[var(--danger)]": !awake(), "bg-[var(--muted)]": awake() && !awakeOk() }} aria-hidden="true" />
        <span class="sr-only">{awake() ? (awakeOk() ? S.status.on : S.status.na) : S.status.off}</span>
      </Button>
      <span class="sb-item" title={memTip()}>
        <Icon name="memory" size={14} />
        <span>{usage() ? fmt(total()) : "–"}</span>
      </span>
      <Show when={usage()?.cpu != null}>
        <span class="sb-item" title={S.status.cpuTip}>
          <Icon name="cpu" size={14} />
          <span>{pct(usage()!.cpu!)}%</span>
        </span>
      </Show>
      <Show when={usage()?.gpu != null}>
        <span class="sb-item">
          <Icon name="gpu" size={14} />
          <span>{pct(usage()!.gpu!)}%</span>
        </span>
      </Show>
      <Show when={ranked(tokens()).length}>
        <span class="sb-item" title={wtUsageTip(tokens())}>
          <span>Σ {fmtTokens(spentOf(totalOf(tokens())))}</span>
        </span>
      </Show>
      <Popover
        label={S.status.agentsRunning(running())}
        chip={
          <>
            <Icon name="terminal" size={14} />
            <span>{running()}</span>
          </>
        }
      >
        {(close) => (
          <For
            each={
              [
                [S.status.running, "working", groups().running],
                [S.status.stopped, "idle", groups().idle],
                [S.status.waiting, "waiting", groups().waiting],
              ] as [string, "working" | "idle" | "waiting", AgentEntry[]][]
            }
          >
            {([title, state, list]) => (
              <div class="sb-group">
                <div class="sb-head">
                  {title} <span class="lim-dim">({list.length})</span>
                </div>
                <For each={list} fallback={<div class="sb-empty">{S.status.none}</div>}>
                  {(a) => (
                    <Button
                      variant="row"
                      size="bare"
                      class="w-full gap-1.5 rounded-sm px-1.5 py-[3px] hover:bg-foreground/10 focus-visible:bg-foreground/10"
                      onClick={() => {
                        close();
                        openSession(a.path, a.id);
                      }}
                    >
                      <StateDot state={state} />
                      <span class="sb-name">{a.name}</span>
                      <span class="lim-dim sb-wt">{a.worktree}</span>
                    </Button>
                  )}
                </For>
              </div>
            )}
          </For>
        )}
      </Popover>
      <Popover
        label={S.status.wtOfProject(counts().current.total)}
        onOpen={loadOverview}
        chip={
          <>
            <Icon name="branch" size={14} />
            <span>{counts().current.total}</span>
          </>
        }
      >
        {() => (
          <>
            <table class="sb-table">
              <thead>
                <tr>
                  <th scope="col">{S.status.worktrees}</th>
                  <th scope="col">{S.status.all}</th>
                  <th scope="col">{S.status.thisProject}</th>
                </tr>
              </thead>
              <tbody>
                <For
                  each={
                    [
                      [S.status.total, "total"],
                      [S.status.dirty, "dirty"],
                      [S.status.ahead, "ahead"],
                      [S.status.behind, "behind"],
                      [S.status.merged, "merged"],
                      [S.status.noAgent, "no_agent"],
                    ] as const
                  }
                >
                  {([label, k]) => (
                    <tr>
                      <th scope="row">{label}</th>
                      <td>{counts().all[k]}</td>
                      <td>{counts().current[k]}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
            <Show when={mergedList(overview()).length}>
              <div class="sb-head">{S.status.mergedHead}</div>
              <ul class="sb-merged">
                <For each={mergedList(overview())}>
                  {(m) => (
                    <li title={m.why}>
                      <span class="sb-name">{m.branch}</span> <span class="lim-dim">{m.project}</span>
                    </li>
                  )}
                </For>
              </ul>
              <Button variant="outline" size="sm" class="mx-1.5 mb-0.5 mt-1" onClick={() => void clean()}>
                {S.status.cleanTitle}
              </Button>
            </Show>
          </>
        )}
      </Popover>
      <span class="sb-item" title={ports().length ? ports().map((p) => `:${p.port}`).join("\n") : S.status.noPorts}>
        <Icon name="plug" size={14} />
        <span>{ports().length}</span>
      </span>
      <Button variant="row" size="xs" class="sb-item cursor-pointer hover:bg-foreground/10 hover:text-foreground" title={K(S.status.shortcutsTip)} aria-label={S.status.shortcutsHint} onClick={() => setShowShortcuts(true)}>
        <kbd>?</kbd>
        <span class="hint-keys">{S.status.shortcutsHint}</span>
      </Button>
      <NotificationBell onJump={(e) => { if (e.session != null && e.worktree) openSession(e.worktree, e.session); else if (e.worktree) selectWorktree(e.worktree); }} />
      <LimitsBar />
      <VersionChip />
    </div>
  );
}
