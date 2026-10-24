import { For, Show } from "solid-js";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  DockerCompose,
  DockerContainer,
  DockerImage,
  DockerNetwork,
  DockerStat,
  DockerStatus,
  DockerVolume,
} from "../api";
import { Badge, Button } from "../ui";
import { ago, dockerError, imageUsers, isRunning, parseCreated, portChips, stateInfo } from "../dockerLogic";
import { DIcon } from "./shared";
import { P } from "../strings.panels";
import { fmtFixed } from "../i18n";

export function Empty(props: { text: string }) {
  return <div class="dk-empty auto-note">{props.text}</div>;
}

export function Unavailable(props: { status: DockerStatus | null; err: string; loaded: boolean; retry: () => void }) {
  return (
    <div class="dk-empty">
      <Show when={props.loaded || props.status} fallback={<span class="auto-note">{P.docker.checking}</span>}>
        <Show
          when={props.status?.installed !== false}
          fallback={
            <>
              <strong>{P.docker.notInstalled}</strong>
              <span class="auto-note">{P.docker.installHint}</span>
              <Button
                variant="link"
                size="bare"
                class="text-primary underline hover:text-primary"
                onClick={() => void openUrl("https://docs.docker.com/get-started/get-docker/")}
              >
                {P.docker.howInstall}
              </Button>
              <Button size="sm" onClick={props.retry}>
                {P.docker.retry}
              </Button>
            </>
          }
        >
          <strong>{P.docker.engineDown}</strong>
          <span class="auto-note">{P.docker.engineHint}</span>
          <Show when={dockerError(props.status) || props.err}>
            <code class="dk-code">{dockerError(props.status) || props.err}</code>
          </Show>
          <Button size="sm" onClick={props.retry}>
            {P.docker.retry}
          </Button>
        </Show>
      </Show>
    </div>
  );
}

export function ComposeBar(props: {
  compose: DockerCompose | null;
  wt: string | null;
  busy: boolean;
  run: (k: "up" | "down") => void;
}) {
  return (
    <Show when={props.wt && props.compose}>
      <div class="dk-compose">
        <Show when={props.compose!.file} fallback={<span class="auto-note">{P.docker.noCompose}</span>}>
          <strong>{P.docker.composeOf}</strong>
          <span class="auto-note">{P.docker.composeProject(props.compose!.file!, props.compose!.project)}</span>
          <For each={props.compose!.services}>
            {(s) => (
              <Badge class="[&_.dot.warn]:bg-[var(--warn)]" title={s.status}>
                <span
                  class="dot"
                  classList={{ on: stateInfo(s.state).cls === "on", warn: stateInfo(s.state).cls === "warn" }}
                />{" "}
                {s.service || s.name} · {stateInfo(s.state).text}
              </Badge>
            )}
          </For>
          <Show when={!props.compose!.services.length}>
            <span class="auto-note">{P.docker.noServices}</span>
          </Show>
          <span class="grow" />
          <Button size="sm" disabled={props.busy} onClick={() => props.run("up")}>
            {P.docker.start}
          </Button>
          <Button size="sm" disabled={props.busy} onClick={() => props.run("down")}>
            {P.docker.composeDownOk}
          </Button>
        </Show>
      </div>
    </Show>
  );
}

export function Row(props: {
  c: DockerContainer;
  stat?: DockerStat;
  sel: boolean;
  cur: boolean;
  busy: boolean;
  onPick: () => void;
  onMenu: (e: MouseEvent) => void;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
  onLogs: () => void;
  onShell: () => void;
  onRemove: () => void;
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
      onClick={() => props.onPick()}
      onContextMenu={(e) => props.onMenu(e)}
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
              <Button
                variant="outline"
                size="bare"
                class="px-1.5 text-xs"
                title={P.docker.openUrl(p.url)}
                aria-label={P.docker.openUrl(p.url)}
                onClick={(e) => {
                  e.stopPropagation();
                  void openUrl(p.url!);
                }}
              >
                {p.label}
              </Button>
            ) : (
              <Badge title={P.docker.portExposed}>{p.label}</Badge>
            )
          }
        </For>
      </span>
      <span class="dk-num" title="CPU">
        {run() && props.stat ? `${fmtFixed(props.stat.cpu, 1)}%` : ""}
      </span>
      <span class="dk-num dk-mem" title={P.docker.memory}>
        {run() && props.stat ? props.stat.mem.split(" / ")[0] : ""}
      </span>
      <span class="dk-num dk-age" title={props.c.created}>
        {ago(parseCreated(props.c.created))}
      </span>
      <span class="dk-acts" role="group" aria-label={P.docker.actions} onClick={(e) => e.stopPropagation()}>
        <Show when={!run()}>
          <Button
            variant="icon"
            size="icon"
            aria-label={P.docker.startNamed(props.c.name)}
            title={P.docker.start}
            disabled={props.busy}
            onClick={props.onStart}
          >
            <DIcon name="play" />
          </Button>
        </Show>
        <Show when={run()}>
          <Button
            variant="icon"
            size="icon"
            aria-label={P.docker.stopNamed(props.c.name)}
            title={P.docker.stop}
            disabled={props.busy}
            onClick={props.onStop}
          >
            <DIcon name="stop" />
          </Button>
          <Button
            variant="icon"
            size="icon"
            aria-label={P.docker.restartNamed(props.c.name)}
            title={P.docker.restart}
            disabled={props.busy}
            onClick={props.onRestart}
          >
            <DIcon name="refresh" />
          </Button>
        </Show>
        <Button
          variant="icon"
          size="icon"
          aria-label={P.docker.logsOf(props.c.name)}
          title={P.docker.menu.logs}
          onClick={props.onLogs}
        >
          <DIcon name="logs" />
        </Button>
        <Button
          variant="icon"
          size="icon"
          aria-label={P.docker.shellIn(props.c.name)}
          title={P.docker.menu.shell}
          disabled={!run()}
          onClick={props.onShell}
        >
          <DIcon name="terminal" />
        </Button>
        <Button
          variant="icon"
          size="icon"
          class="text-destructive"
          aria-label={P.docker.removeNamed(props.c.name)}
          title={P.docker.remove}
          disabled={props.busy}
          onClick={props.onRemove}
        >
          <DIcon name="trash" />
        </Button>
      </span>
    </div>
  );
}

const has = (q: string, ...parts: string[]) => {
  const hay = parts.join(" ").toLowerCase();
  return q
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((w) => hay.includes(w));
};

export function VolumesList(props: {
  list: DockerVolume[];
  q: string;
  busy: boolean;
  onRemove: (v: DockerVolume) => void;
}) {
  const rows = () => props.list.filter((v) => has(props.q, v.name, v.driver, ...v.used_by));
  return (
    <Show
      when={rows().length}
      fallback={<Empty text={props.list.length ? P.docker.noVolumeMatch : P.docker.noVolumes} />}
    >
      <div class="dk-list">
        <For each={rows()}>
          {(v) => (
            <div class="dk-row plain">
              <span class="dk-name wide">
                <strong title={v.name}>{v.name}</strong>
                <span class="auto-note" title={v.mountpoint}>
                  {v.driver} · {v.mountpoint}
                </span>
              </span>
              <span class="dk-ports">
                <Show when={v.used_by.length} fallback={<Badge variant="warn">{P.docker.unused}</Badge>}>
                  <For each={v.used_by}>{(n) => <Badge>{n}</Badge>}</For>
                </Show>
              </span>
              <span class="dk-acts show">
                <Button
                  variant="icon"
                  size="icon"
                  class="text-destructive"
                  aria-label={P.docker.removeVolumeNamed(v.name)}
                  title={P.docker.remove}
                  disabled={props.busy || v.used_by.length > 0}
                  onClick={() => props.onRemove(v)}
                >
                  <DIcon name="trash" />
                </Button>
              </span>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}

export function ImagesList(props: {
  list: DockerImage[];
  containers: DockerContainer[];
  q: string;
  busy: boolean;
  onRemove: (i: DockerImage) => void;
}) {
  const rows = () => props.list.filter((i) => has(props.q, i.id, ...i.tags));
  return (
    <Show
      when={rows().length}
      fallback={<Empty text={props.list.length ? P.docker.noImageMatch : P.docker.noImages} />}
    >
      <div class="dk-list">
        <For each={rows()}>
          {(i) => {
            const users = () => imageUsers(i, props.containers);
            return (
              <div class="dk-row plain">
                <span class="dk-name wide">
                  <strong>{i.tags.length ? i.tags.join(", ") : P.docker.unnamed}</strong>
                  <span class="auto-note">
                    {i.id} · {ago(parseCreated(i.created))}
                  </span>
                </span>
                <span class="dk-ports">
                  <Show when={i.dangling}>
                    <Badge variant="warn" title={P.docker.orphanTip}>
                      {P.docker.orphan}
                    </Badge>
                  </Show>
                  <Show when={users().length} fallback={<Badge>{P.docker.noContainers}</Badge>}>
                    <For each={users()}>{(c) => <Badge>{c.name}</Badge>}</For>
                  </Show>
                </span>
                <span class="dk-num dk-size">{i.size}</span>
                <span class="dk-acts show">
                  <Button
                    variant="icon"
                    size="icon"
                    class="text-destructive"
                    aria-label={P.docker.removeImageNamed(i.tags[0] ?? i.id)}
                    title={P.docker.remove}
                    disabled={props.busy || users().length > 0}
                    onClick={() => props.onRemove(i)}
                  >
                    <DIcon name="trash" />
                  </Button>
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
    <Show
      when={rows().length}
      fallback={<Empty text={props.list.length ? P.docker.noNetworkMatch : P.docker.noNetworks} />}
    >
      <div class="dk-list">
        <For each={rows()}>
          {(n) => (
            <div class="dk-row plain">
              <span class="dk-name wide">
                <strong>{n.name}</strong>
                <span class="auto-note">
                  {n.driver} · {n.scope} · {n.id.slice(0, 12)}
                </span>
              </span>
              <span class="dk-ports">
                <Show when={n.containers.length} fallback={<Badge>{P.docker.noContainers}</Badge>}>
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
