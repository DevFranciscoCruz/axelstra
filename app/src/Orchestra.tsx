import { For, Show, createResource, createSignal } from "solid-js";
import { SessionInfo, killSession, removeSession, workerLimits } from "./api";
import { ask } from "./Confirm";
import { Modal } from "./Modal";
import { openSession, refreshSessions, sessionState, sessions, setShowOrchestra } from "./store";

const baseName = (p?: string) => (p ?? "").replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "";
const STATE: Record<string, string> = { working: "a trabalhar", waiting: "espera resposta", idle: "parado", done: "terminou" };

/** coordinators and the workers they started, with their state: the picture behind /axelstration */
export function Orchestra() {
  const [limits, { refetch }] = createResource(() => workerLimits().catch(() => null));
  const [max, setMax] = createSignal<number | null>(null);

  const kids = (id: number) => sessions().filter((s) => s.meta?.parent === id);
  const known = (id: number) => sessions().some((s) => s.id === id);
  // sessions that have workers, and workers whose parent is gone
  const roots = () => sessions().filter((s) => kids(s.id).length > 0 && !(s.meta?.parent && known(s.meta.parent)));
  const orphans = () => sessions().filter((s) => s.meta?.parent && !known(s.meta.parent));
  const workerCount = () => sessions().filter((s) => s.meta?.worker && s.running).length;

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
            {props.depth > 0 ? "↳ " : ""}
            {props.s.label ?? `Sessão ${props.s.id}`}
          </span>
          <span class="auto-note">
            {baseName(props.s.meta?.worktree)} · {props.s.escalated ? "pediu ajuda" : STATE[st()]}
          </span>
          <Show when={(props.s.unread ?? 0) > 0}>
            <span class="wt-badge" title="Mensagens por ler">✉ {props.s.unread}</span>
          </Show>
          <span class="grow" />
          <button onClick={() => open(props.s)} disabled={!props.s.meta?.worktree}>
            Abrir
          </button>
          <Show when={props.depth > 0}>
            <button class="danger" onClick={() => void stop(props.s)}>
              Parar
            </button>
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
        <input
          class="orch-limit"
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
        <Show when={roots().length === 0 && orphans().length === 0}>
          <div class="auto-note">
            Ainda não há workers. Peça a um agente (Claude, Codex…) para usar <b>/axelstration</b> e eles aparecem aqui, por baixo de quem os lançou.
          </div>
        </Show>
      </div>
    </Modal>
  );
}
