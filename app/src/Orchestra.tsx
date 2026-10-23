import { For, Show, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { Icon } from "./Icon";
import { SessionInfo, SessionRes, killSession, removeSession, sessionResources, workerLimits } from "./api";
import { ask } from "./Confirm";
import { Modal } from "./Modal";
import { Button, Input } from "./ui";
import { openSession, refreshSessions, sessionState, sessions, setShowOrchestra } from "./store";

const baseName = (p?: string) => (p ?? "").replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "";
const STATE: Record<string, string> = { working: "a trabalhar", waiting: "espera resposta", idle: "parado", done: "terminou" };

/** coordinators and the workers they started, with their state: the picture behind /axelstration */
export function Orchestra() {
  const [limits, { refetch }] = createResource(() => workerLimits().catch(() => null));
  const [max, setMax] = createSignal<number | null>(null);
  // memory now and CPU (% of the machine, from the change in CPU time between two readings) of each session
  const [res, setRes] = createSignal<Record<number, { mem: number; cpu: number | null }>>({});
  let prev: { at: number; cpu: Record<number, number> } | null = null;
  const pollRes = async () => {
    if (document.hidden) return;
    try {
      const r = await sessionResources();
      const at = Date.now();
      const out: Record<number, { mem: number; cpu: number | null }> = {};
      for (const s of r.sessions as SessionRes[]) {
        const before = prev?.cpu[s.id];
        const dt = prev ? (at - prev.at) / 1000 : 0;
        out[s.id] = { mem: s.mem, cpu: before !== undefined && dt > 0.5 ? Math.min(100, Math.max(0, (s.cpu_100ns - before) / 1e7 / dt / r.cores) * 100) : null };
      }
      prev = { at, cpu: Object.fromEntries(r.sessions.map((s) => [s.id, s.cpu_100ns])) };
      setRes(out);
    } catch {
      /* an older daemon has no session.resources */
    }
  };
  onMount(() => {
    void pollRes();
    const t = setInterval(() => void pollRes(), 3000);
    onCleanup(() => clearInterval(t));
  });
  const mb = (n: number) => (n >= 1024 ** 3 ? `${(n / 1024 ** 3).toFixed(1)} GB` : `${Math.round(n / 1024 ** 2)} MB`);

  const kids = (id: number) => sessions().filter((s) => s.meta?.parent === id);
  const known = (id: number) => sessions().some((s) => s.id === id);
  // sessions that have workers, and workers whose parent is gone
  const roots = () => sessions().filter((s) => kids(s.id).length > 0 && !(s.meta?.parent && known(s.meta.parent)));
  const orphans = () => sessions().filter((s) => s.meta?.parent && !known(s.meta.parent));
  // agents that neither lead workers nor are workers: shown too, so the window is the board for every agent
  const loners = () => sessions().filter((s) => s.meta?.kind === "agent" && !s.meta?.parent && kids(s.id).length === 0);
  const since = (ms?: number) => {
    const m = Math.floor((ms ?? 0) / 60000);
    return m < 1 ? "" : m < 60 ? ` há ${m} min` : ` há ${Math.floor(m / 60)} h`;
  };
  const workerCount =() => sessions().filter((s) => s.meta?.worker && s.running).length;

  const open = (s: SessionInfo) => {
    const wt = s.meta?.worktree;
    if (!wt) return;
    setShowOrchestra(false);
    openSession(wt, s.id);
  };
  const stop = async (s: SessionInfo) => {
    if (!(await ask(`Parar "${s.label ?? `Sessão ${s.id}`}"? O que ele estiver a fazer é terminado.`, { title: "Parar worker", kind: "warning", okLabel: "Parar", cancelLabel: "Cancelar" }))) return;
    if (s.running) await killSession(s.id).catch(() => 0);
    await removeSession(s.id).catch(() => 0);
    await refreshSessions();
  };
  const applyMax = async () => {
    const n = max();
    if (n === null || n < 1) return;
    await workerLimits(n).catch(() => 0);
    setMax(null);
    void refetch();
  };

  const Row = (props: { s: SessionInfo; depth: number }) => {
    const st = () => sessionState(props.s);
    return (
      <>
        <div class="orch-row" style={{ "padding-left": `${props.depth * 18}px` }}>
          <span class="dot" classList={{ work: st() === "working", wait: st() === "waiting", on: st() === "idle" }} />
          <span class="orch-name">
            {props.depth > 0 ? <Icon name="subArrow" size={11} inline /> : null}
            {props.s.label ?? `Sessão ${props.s.id}`}
          </span>
          <span class="auto-note">
            {baseName(props.s.meta?.worktree)} · {props.s.escalated ? "pediu ajuda" : STATE[st()]}
            {st() === "working" ? "" : since(props.s.idle_ms)}
          </span>
          <Show when={res()[props.s.id]}>
            <span class="auto-note" title="Memória e CPU de tudo o que esta sessão corre (do computador inteiro)">
              {mb(res()[props.s.id].mem)}
              {res()[props.s.id].cpu !== null ? ` · ${res()[props.s.id].cpu!.toFixed(res()[props.s.id].cpu! < 10 ? 1 : 0)}% CPU` : ""}
            </span>
          </Show>
          <Show when={(props.s.unread ?? 0) > 0}>
            <span class="wt-badge" title="Mensagens por ler"><Icon name="mail" size={11} inline /> {props.s.unread}</span>
          </Show>
          <span class="grow" />
          <Button size="sm" onClick={() => open(props.s)} disabled={!props.s.meta?.worktree}>
            Abrir
          </Button>
          <Show when={props.depth > 0}>
            <Button size="sm" variant="destructive" class="danger" onClick={() => void stop(props.s)}>
              Parar
            </Button>
          </Show>
        </div>
        <For each={kids(props.s.id)}>{(k) => <Row s={k} depth={props.depth + 1} />}</For>
      </>
    );
  };

  return (
    <Modal label="Orquestração" onClose={() => setShowOrchestra(false)} class="modal auto">
      <div class="m-title" style={{ "font-weight": "600" }}>Orquestração</div>
      <div class="orch-head auto-note">
        {workerCount()} {workerCount() === 1 ? "worker a correr" : "workers a correr"} · limite
        <Input
          class="orch-limit w-16"
          type="number"
          min="1"
          max="64"
          value={max() ?? limits()?.max_workers ?? 6}
          onInput={(e) => setMax(Number(e.currentTarget.value))}
          onKeyDown={(e) => e.key === "Enter" && void applyMax()}
          onBlur={() => void applyMax()}
        />
      </div>
      <div class="auto-list">
        <For each={roots()}>{(s) => <Row s={s} depth={0} />}</For>
        <For each={orphans()}>{(s) => <Row s={s} depth={1} />}</For>
        <Show when={loners().length > 0}>
          <div class="auto-note" style={{ "margin-top": "10px" }}>Outros agentes</div>
          <For each={loners()}>{(s) => <Row s={s} depth={0} />}</For>
        </Show>
        <Show when={roots().length === 0 && orphans().length === 0 && loners().length === 0}>
          <div class="auto-note">
            Ainda não há workers. Pede a um agente (Claude, Codex…) para usar <b>/axelstration</b> e eles aparecem aqui, por baixo de quem os lançou.
          </div>
        </Show>
      </div>
    </Modal>
  );
}
