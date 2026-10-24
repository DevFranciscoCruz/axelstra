import { For, JSX, Show, on } from "solid-js";
import { openUrl } from "@tauri-apps/plugin-opener";
import { DockerCompose, DockerContainer, DockerImage, DockerNetwork, DockerStat, DockerStatus, DockerVolume } from "../api";
import { Icon } from "../Icon";
import { Badge, Button } from "../ui";
import { ago, imageUsers, isRunning, parseCreated, portChips, stateInfo } from "../dockerLogic";
import { sessions, setError, showNotice } from "../store";
import { DIcon } from "./shared";

export function Empty(props: { text: string }) {
  return <div class="dk-empty auto-note">{props.text}</div>;
}

export function Unavailable(props: { status: DockerStatus | null; err: string; loaded: boolean; retry: () => void }) {
  return (
    <div class="dk-empty">
      <Show when={props.loaded || props.status} fallback={<span class="auto-note">A verificar o Docker…</span>}>
        <Show
          when={props.status?.installed !== false}
          fallback={
            <>
              <strong>O Docker não está instalado</strong>
              <span class="auto-note">Instale o Docker Desktop para ver e gerir contentores aqui. Depois de instalar, abra-o e volte a esta vista.</span>
              <Button variant="link" size="bare" class="text-primary underline hover:text-primary" onClick={() => void openUrl("https://docs.docker.com/get-started/get-docker/")}>
                Como instalar o Docker Desktop
              </Button>
              <Button size="sm" onClick={props.retry}>Tentar de novo</Button>
            </>
          }
        >
          <strong>O motor do Docker não está a correr</strong>
          <span class="auto-note">Abra o Docker Desktop e espere que fique pronto. Esta vista volta a tentar sozinha a cada 10 segundos.</span>
          <Show when={props.status?.error || props.err}>
            <code class="dk-code">{props.status?.error || props.err}</code>
          </Show>
          <Button size="sm" onClick={props.retry}>Tentar de novo</Button>
        </Show>
      </Show>
    </div>
  );
}

export function ComposeBar(props: { compose: DockerCompose | null; wt: string | null; busy: boolean; run: (k: "up" | "down") => void }) {
  return (
    <Show when={props.wt && props.compose}>
      <div class="dk-compose">
        <Show when={props.compose!.file} fallback={<span class="auto-note">Este worktree não tem ficheiro compose (compose.yaml, docker-compose.yml…).</span>}>
          <strong>Compose deste worktree</strong>
          <span class="auto-note">{props.compose!.file} · projeto {props.compose!.project}</span>
          <For each={props.compose!.services}>
            {(s) => (
              <Badge class="[&_.dot.warn]:bg-[var(--warn)]" title={s.status}>
                <span class="dot" classList={{ on: stateInfo(s.state).cls === "on", warn: stateInfo(s.state).cls === "warn" }} /> {s.service || s.name} · {stateInfo(s.state).text}
              </Badge>
            )}
          </For>
          <Show when={!props.compose!.services.length}>
            <span class="auto-note">Sem serviços criados.</span>
          </Show>
          <span class="grow" />
          <Button size="sm" disabled={props.busy} onClick={() => props.run("up")}>Iniciar</Button>
          <Button size="sm" disabled={props.busy} onClick={() => props.run("down")}>Parar e remover</Button>
        </Show>
      </div>
    </Show>
  );
}

export function Row(props: {
  c: DockerContainer; stat?: DockerStat; sel: boolean; cur: boolean; busy: boolean;
  onPick: () => void; onMenu: (e: MouseEvent) => void; onStart: () => void; onStop: () => void; onRestart: () => void; onLogs: () => void; onShell: () => void; onRemove: () => void;
}) {
  const st = () => stateInfo(props.c.state);
  const chips = () => portChips(props.c.ports);
  const run = () => isRunning(props.c);
  return (
    <div
      class="dk-row"
      classList={{ sel: props.sel, cur: props.cur }}
      role="option"
      aria-selected={props.sel}
      tabindex="-1"
      data-cid={props.c.id}
      onClick={props.onPick}
      onContextMenu={props.onMenu}
    >
      <span class="dk-state" title={props.c.status}>
        <span class="dot" classList={{ on: st().cls === "on", warn: st().cls === "warn" }} />
        <span>{st().text}</span>
      </span>
      <span class="dk-name">
        <strong>{props.c.name}</strong>
        <span class="auto-note">{props.c.image}</span>
      </span>
      <span class="dk-ports">
        <For each={chips()}>
          {(p) =>
            p.url ? (
              <Button variant="outline" size="bare" class="px-1.5 text-xs" title={`Abrir ${p.url} no navegador`} aria-label={`Abrir ${p.url} no navegador`} onClick={(e) => { e.stopPropagation(); void openUrl(p.url!); }}>
                {p.label}
              </Button>
            ) : (
              <Badge title="Porta exposta, não publicada">{p.label}</Badge>
            )
          }
        </For>
      </span>
      <span class="dk-num" title="CPU">{run() && props.stat ? `${props.stat.cpu.toFixed(1)}%` : ""}</span>
      <span class="dk-num dk-mem" title="Memória">{run() && props.stat ? props.stat.mem.split(" / ")[0] : ""}</span>
      <span class="dk-num dk-age" title={props.c.created}>{ago(parseCreated(props.c.created))}</span>
      <span class="dk-acts" role="group" aria-label="Ações" onClick={(e) => e.stopPropagation()}>
        <Show when={!run()}>
          <Button variant="icon" size="icon" aria-label={`Iniciar ${props.c.name}`} title="Iniciar" disabled={props.busy} onClick={props.onStart}><DIcon name="play" /></Button>
        </Show>
        <Show when={run()}>
          <Button variant="icon" size="icon" aria-label={`Parar ${props.c.name}`} title="Parar" disabled={props.busy} onClick={props.onStop}><DIcon name="stop" /></Button>
          <Button variant="icon" size="icon" aria-label={`Reiniciar ${props.c.name}`} title="Reiniciar" disabled={props.busy} onClick={props.onRestart}><DIcon name="refresh" /></Button>
        </Show>
        <Button variant="icon" size="icon" aria-label={`Logs de ${props.c.name}`} title="Logs" onClick={props.onLogs}><DIcon name="logs" /></Button>
        <Button variant="icon" size="icon" aria-label={`Shell em ${props.c.name}`} title="Shell" disabled={!run()} onClick={props.onShell}><DIcon name="terminal" /></Button>
        <Button variant="icon" size="icon" class="text-destructive" aria-label={`Remover ${props.c.name}`} title="Remover" disabled={props.busy} onClick={props.onRemove}><DIcon name="trash" /></Button>
      </span>
    </div>
  );
}

const has = (q: string, ...parts: string[]) => {
  const hay = parts.join(" ").toLowerCase();
  return q.toLowerCase().split(/\s+/).filter(Boolean).every((w) => hay.includes(w));
};

export function VolumesList(props: { list: DockerVolume[]; q: string; busy: boolean; onRemove: (v: DockerVolume) => void }) {
  const rows = () => props.list.filter((v) => has(props.q, v.name, v.driver, ...v.used_by));
  return (
    <Show when={rows().length} fallback={<Empty text={props.list.length ? "Nenhum volume corresponde à pesquisa." : "Não há volumes."} />}>
      <div class="dk-list">
        <For each={rows()}>
          {(v) => (
            <div class="dk-row plain">
              <span class="dk-name wide">
                <strong title={v.name}>{v.name}</strong>
                <span class="auto-note" title={v.mountpoint}>{v.driver} · {v.mountpoint}</span>
              </span>
              <span class="dk-ports">
                <Show when={v.used_by.length} fallback={<Badge variant="warn">sem uso</Badge>}>
                  <For each={v.used_by}>{(n) => <Badge>{n}</Badge>}</For>
                </Show>
              </span>
              <span class="dk-acts show">
                <Button variant="icon" size="icon" class="text-destructive" aria-label={`Remover volume ${v.name}`} title="Remover" disabled={props.busy || v.used_by.length > 0} onClick={() => props.onRemove(v)}><DIcon name="trash" /></Button>
              </span>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}

export function ImagesList(props: { list: DockerImage[]; containers: DockerContainer[]; q: string; busy: boolean; onRemove: (i: DockerImage) => void }) {
  const rows = () => props.list.filter((i) => has(props.q, i.id, ...i.tags));
  return (
    <Show when={rows().length} fallback={<Empty text={props.list.length ? "Nenhuma imagem corresponde à pesquisa." : "Não há imagens."} />}>
      <div class="dk-list">
        <For each={rows()}>
          {(i) => {
            const users = () => imageUsers(i, props.containers);
            return (
              <div class="dk-row plain">
                <span class="dk-name wide">
                  <strong>{i.tags.length ? i.tags.join(", ") : "<sem nome>"}</strong>
                  <span class="auto-note">{i.id} · {ago(parseCreated(i.created))}</span>
                </span>
                <span class="dk-ports">
                  <Show when={i.dangling}><Badge variant="warn" title="Sem nome nem etiqueta: sobra de uma reconstrução">dangling</Badge></Show>
                  <Show when={users().length} fallback={<Badge>sem contentores</Badge>}>
                    <For each={users()}>{(c) => <Badge>{c.name}</Badge>}</For>
                  </Show>
                </span>
                <span class="dk-num dk-size">{i.size}</span>
                <span class="dk-acts show">
                  <Button variant="icon" size="icon" class="text-destructive" aria-label={`Remover imagem ${i.tags[0] ?? i.id}`} title="Remover" disabled={props.busy || users().length > 0} onClick={() => props.onRemove(i)}><DIcon name="trash" /></Button>
                </span>
              </div>
            );
          }}
        </For>
      </div>
    </Show>
  );
}

export function NetworksList(props: { list: DockerNetwork[]; q: string }) {
  const rows = () => props.list.filter((n) => has(props.q, n.name, n.driver, ...n.containers));
  return (
    <Show when={rows().length} fallback={<Empty text={props.list.length ? "Nenhuma rede corresponde à pesquisa." : "Não há redes."} />}>
      <div class="dk-list">
        <For each={rows()}>
          {(n) => (
            <div class="dk-row plain">
              <span class="dk-name wide">
                <strong>{n.name}</strong>
                <span class="auto-note">{n.driver} · {n.scope} · {n.id.slice(0, 12)}</span>
              </span>
              <span class="dk-ports">
                <Show when={n.containers.length} fallback={<Badge>sem contentores</Badge>}>
                  <For each={n.containers}>{(c) => <Badge>{c}</Badge>}</For>
                </Show>
              </span>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
