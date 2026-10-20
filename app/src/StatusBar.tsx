import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { sessions } from "./store";
import { LimitsBar } from "./LimitsBar";

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

/** Thin bar under the whole window: keep-awake, memory used by Axelstra, running terminals, dev-server ports. */
export function StatusBar() {
  const [awake, setAwake] = createSignal(stored());
  const [usage, setUsage] = createSignal<Usage | null>(null);
  const running = () => sessions().filter((s) => s.running).length;

  const toggle = () => {
    const on = !awake();
    setAwake(on);
    try {
      localStorage.setItem(KEY, on ? "1" : "0");
    } catch {
      /* storage unavailable */
    }
    void invoke("keep_awake", { on }).catch(() => 0);
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
    if (awake()) void invoke("keep_awake", { on: true }).catch(() => 0);
    void poll();
    const t = setInterval(() => void poll(), 5000);
    const vis = () => void poll();
    document.addEventListener("visibilitychange", vis);
    onCleanup(() => {
      clearInterval(t);
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
  const gpuTip = () => {
    const u = usage();
    const per = (u?.gpu_adapters ?? []).map(([id, v]) => `${id}: ${pct(v)}%`).join("\n");
    return `GPU usada pelo Axelstra (app, WebView2 e terminais), no motor mais ocupado${per ? "\n" + per : ""}`;
  };

  return (
    <div class="statusbar">
      <button class="sb-item" classList={{ on: awake() }} title="Impedir que o computador adormeça" aria-pressed={awake()} onClick={toggle}>
        <svg {...svg}>
          <path d="M3 7h8v3a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V7zM11 8h1a1.5 1.5 0 0 1 0 3h-1M5 2.5v2M8 2.5v2" />
        </svg>
        <span>{awake() ? "On" : "Off"}</span>
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
        <span class="sb-item" title={gpuTip()}>
          <svg {...svg}>
            <rect x="1.5" y="4.5" width="13" height="7" rx="1" />
            <circle cx="8" cy="8" r="2" />
            <path d="M4 11.5v2M12 11.5v2" />
          </svg>
          <span>{pct(usage()!.gpu!)}%</span>
        </span>
      </Show>
      <span class="sb-item" title={`${running()} terminais e agentes a correr`}>
        <svg {...svg}>
          <path d="M2.5 4.5l3 3-3 3M7.5 11.5h6" />
        </svg>
        <span>{running()}</span>
      </span>
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
