import { For, Show, createEffect, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { findWorktree, openSession, refreshProjects, selectedWorktree, sessionState, sessions, showNotice, tabLabel } from "./store";
import { worktreeOverview, worktreeSweep } from "./api";
import { ask } from "./Confirm";
import { StateDot } from "./StateDot";
import { Icon } from "./Icon";
import { countsFor, groupAgents, mergedList, type AgentEntry, type OverviewProject } from "./statusGroups";
import { LimitsBar } from "./LimitsBar";
import { pollWtUsage, wtUsageTip } from "./Usage";
import { ranked, spentOf, totalOf, fmtTokens } from "./usageFormat";

type Usage = { app: number; daemon: number; sessions: number; ports: { port: number; pid: number }[]; cpu: number | null; gpu: number | null; gpu_adapters: [string, number][] };

const KEY = "axelstra.keepAwake";
const stored = () => {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
};

const fmt = (b: number) => (b >= 1024 ** 3 ? `${(b / 1024 ** 3).toFixed(2)} GB` : `${Math.round(b / 1024 ** 2)} MB`);

const svg = { width: 14, height: 14, viewBox: "0 0 16 16", fill: "none", stroke: "currentColor", "stroke-width": 1.3, "stroke-linecap": "round", "stroke-linejoin": "round" } as const;

/** A status bar chip that opens a popover: a button (keyboard and screen readers), closed by Escape, a click outside or leaving it. */
function Popover(props: { chip: JSX.Element; label: string; onOpen?: () => void; children: (close: () => void) => JSX.Element }) {
  const [open, setOpen] = createSignal(false);
  let box!: HTMLSpanElement;
  const close = () => setOpen(false);
  createEffect(() => {
    if (!open()) return;
    props.onOpen?.();
    const away = (e: Event) => !box.contains(e.target as Node) && close();
    document.addEventListener("pointerdown", away);
    onCleanup(() => document.removeEventListener("pointerdown", away));
  });
  return (
    <span
      class="sb-pop-wrap"
      ref={box}
      onKeyDown={(e) => {
        if (e.key === "Escape" && open()) {
          e.stopPropagation();
          close();
          box.querySelector<HTMLElement>(".sb-item")?.focus();
        }
      }}
      onFocusOut={(e) => !box.contains(e.relatedTarget as Node | null) && close()}
    >
      <button class="sb-item" aria-haspopup="true" aria-expanded={open()} aria-label={props.label} onClick={() => setOpen(!open())}>
        {props.chip}
      </button>
      <Show when={open()}>
        <div class="sb-pop" role="group" aria-label={props.label}>
          {props.children(close)}
        </div>
      </Show>
    </span>
  );
}

/** Thin bar under the whole window: keep-awake, memory used by Axelstra, running terminals, dev-server ports. */
export function StatusBar() {
  const [awake, setAwake] = createSignal(stored());
  const [usage, setUsage] = createSignal<Usage | null>(null);
  const groups = () => groupAgents(sessions(), sessionState, tabLabel);
  const running = () => groups().running.length;
  const [overview, setOverview] = createSignal<OverviewProject[]>([]);
  const currentProject = () => findWorktree(selectedWorktree())?.project.id ?? null;
  const counts = () => countsFor(overview(), currentProject());
  const loadOverview = () => void worktreeOverview().then(setOverview).catch(() => 0);
  const clean = async () => {
    const list = mergedList(overview());
    if (!list.length) return;
    const ok = await ask(`Remover ${list.length} worktree(s) já merged, de todos os projetos?\n\n${list.map((m) => `${m.project}: ${m.branch}`).join("\n")}`, { title: "Limpar merged", kind: "warning", okLabel: "Remover", cancelLabel: "Cancelar" });
    if (!ok) return;
    try {
      const done = (await worktreeSweep()).filter((x) => x.removed).length;
      showNotice(done === 1 ? "1 worktree removida" : `${done} worktrees removidas`);
    } catch (e) {
      showNotice(String(e));
    }
    await refreshProjects();
    loadOverview();
  };
  const tokens = pollWtUsage();
  const [awakeOk, setAwakeOk] = createSignal(true);

  const toggle = () => {
    const on = !awake();
    setAwake(on);
    try {
      localStorage.setItem(KEY, on ? "1" : "0");
    } catch {
      /* storage unavailable */
    }
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
    void poll();
    loadOverview();
    const t = setInterval(() => void poll(), 5000);
    const t2 = setInterval(() => !document.hidden && loadOverview(), 15000);
    const vis = () => void poll();
    document.addEventListener("visibilitychange", vis);
    onCleanup(() => {
      clearInterval(t);
      clearInterval(t2);
      document.removeEventListener("visibilitychange", vis);
    });
  });

  const total = () => {
    const u = usage();
    return u ? u.app + u.daemon + u.sessions : 0;
  };
  const memTip = () => {
    const u = usage();
    if (!u) return "Memória usada pelo Axelstra";
    return `Memória do Axelstra\nAplicação: ${fmt(u.app)}\nDaemon: ${fmt(u.daemon)}\nTerminais e agentes: ${fmt(u.sessions)}`;
  };
  const ports = () => usage()?.ports ?? [];
  const pct = (n: number) => (n < 10 ? n.toFixed(1) : String(Math.round(n)));

  return (
    <div class="statusbar">
      <button class="sb-item" classList={{ on: awake() }} title={awakeOk() ? "Impedir que o computador adormeça" : "Não disponível neste sistema: falta o systemd-inhibit"} aria-pressed={awake()} onClick={toggle}>
        <svg {...svg}>
          <path d="M3 7h8v3a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V7zM11 8h1a1.5 1.5 0 0 1 0 3h-1M5 2.5v2M8 2.5v2" />
        </svg>
        <span>{awake() ? (awakeOk() ? "Ligado" : "n/d") : "Desligado"}</span>
      </button>
      <span class="sb-item" title={memTip()}>
        <svg {...svg}>
          <rect x="2" y="5" width="12" height="6" rx="1" />
          <path d="M5 11v2M8 11v2M11 11v2M5 5V3M8 5V3M11 5V3" />
        </svg>
        <span>{usage() ? fmt(total()) : "–"}</span>
      </span>
      <Show when={usage()?.cpu != null}>
        <span class="sb-item" title="CPU usada pelo Axelstra (app, daemon, terminais e agentes), em % do computador">
          <svg {...svg}>
            <rect x="4" y="4" width="8" height="8" rx="1" />
            <path d="M6.5 1.5v2.5M9.5 1.5v2.5M6.5 12v2.5M9.5 12v2.5M1.5 6.5H4M1.5 9.5H4M12 6.5h2.5M12 9.5h2.5" />
          </svg>
          <span>{pct(usage()!.cpu!)}%</span>
        </span>
      </Show>
      <Show when={usage()?.gpu != null}>
        <span class="sb-item">
          <svg {...svg}>
            <rect x="1.5" y="4.5" width="13" height="7" rx="1" />
            <circle cx="8" cy="8" r="2" />
            <path d="M4 11.5v2M12 11.5v2" />
          </svg>
          <span>{pct(usage()!.gpu!)}%</span>
        </span>
      </Show>
      <Show when={ranked(tokens()).length}>
        <span class="sb-item" title={wtUsageTip(tokens())}>
          <span>Σ {fmtTokens(spentOf(totalOf(tokens())))}</span>
        </span>
      </Show>
      <Popover
        label={`${running()} agentes a correr`}
        chip={
          <>
            <svg {...svg}>
              <path d="M2.5 4.5l3 3-3 3M7.5 11.5h6" />
            </svg>
            <span>{running()}</span>
          </>
        }
      >
        {(close) => (
          <For
            each={
              [
                ["A correr", "working", groups().running],
                ["Parados", "idle", groups().idle],
                ["A aguardar resposta", "waiting", groups().waiting],
              ] as [string, "working" | "idle" | "waiting", AgentEntry[]][]
            }
          >
            {([title, state, list]) => (
              <div class="sb-group">
                <div class="sb-head">
                  {title} <span class="lim-dim">({list.length})</span>
                </div>
                <For each={list} fallback={<div class="sb-empty">Nenhum</div>}>
                  {(a) => (
                    <button
                      class="sb-entry"
                      onClick={() => {
                        close();
                        openSession(a.path, a.id);
                      }}
                    >
                      <StateDot state={state} />
                      <span class="sb-name">{a.name}</span>
                      <span class="lim-dim sb-wt">{a.worktree}</span>
                    </button>
                  )}
                </For>
              </div>
            )}
          </For>
        )}
      </Popover>
      <Popover
        label={`${counts().current.total} worktrees do projeto atual`}
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
                  <th scope="col">Worktrees</th>
                  <th scope="col">Todos</th>
                  <th scope="col">Este projeto</th>
                </tr>
              </thead>
              <tbody>
                <For
                  each={
                    [
                      ["Total", "total"],
                      ["Com alterações por guardar", "dirty"],
                      ["Commits à frente do remoto", "ahead"],
                      ["Atrás do remoto", "behind"],
                      ["Já merged (seguras de remover)", "merged"],
                      ["Sem agente a correr", "no_agent"],
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
              <div class="sb-head">Já merged</div>
              <ul class="sb-merged">
                <For each={mergedList(overview())}>
                  {(m) => (
                    <li title={m.why}>
                      <span class="sb-name">{m.branch}</span> <span class="lim-dim">{m.project}</span>
                    </li>
                  )}
                </For>
              </ul>
              <button class="sb-clean" onClick={() => void clean()}>
                Limpar merged
              </button>
            </Show>
          </>
        )}
      </Popover>
      <span class="sb-item" title={ports().length ? ports().map((p) => `:${p.port}`).join("\n") : "Nenhuma porta de servidor aberta"}>
        <svg {...svg}>
          <path d="M6 2.5v3M10 2.5v3M4.5 5.5h7v2.5a3.5 3.5 0 0 1-7 0V5.5zM8 11.5v2" />
        </svg>
        <span>{ports().length}</span>
      </span>
      <LimitsBar />
    </div>
  );
}
