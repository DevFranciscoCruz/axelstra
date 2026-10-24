import { For, JSX, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  DockerAction, DockerCompose, DockerContainer, DockerFsEntry, DockerImage, DockerInspect, DockerNetwork, DockerStat, DockerStatus, DockerVolume,
  dockerAction, dockerContainers, dockerFsList, dockerFsRead, dockerImages, dockerInspect, dockerLogs, dockerNetworks, dockerOpenLogs, dockerOpenShell,
  dockerStats, dockerStatus, dockerVolumes,
} from "./api";
import { ask } from "./Confirm";
import { MenuItem, isMenuKey, showMenu, showMenuAt } from "./ContextMenu";
import { Icon } from "./Icon";
import { Badge, Button, Checkbox, Input, cn } from "./ui";
import { TerminalView } from "./TerminalView";
import { Filter, ago, filterContainers, groupByProject, imageUsers, isRunning, isSecretKey, joinPath, parseCreated, portChips, searchLog, stateInfo } from "./dockerLogic";
import { findWorktree, keepSame, openSession, refreshSessions, selectedWorktree, sessions, setError, setShowDocker, showDocker, showNotice } from "./store";
import "./docker.css";

/** The Docker view fills the centre area like the Kanban; whoever mounts it owns this signal. */
export { showDocker, setShowDocker };

type Nav = "containers" | "volumes" | "images" | "networks";
type DTab = "info" | "logs" | "terminal" | "files";
const NAV: [Nav, string][] = [["containers", "Contentores"], ["volumes", "Volumes"], ["images", "Imagens"], ["networks", "Redes"]];
const DTABS: [DTab, string][] = [["info", "Info"], ["logs", "Logs"], ["terminal", "Terminal"], ["files", "Ficheiros"]];
const FAST_MS = 3000;
const SLOW_MS = 10000;

/** Small stroke icons the shared Icon set does not have. */
const GLYPHS: Record<string, JSX.Element> = {
  stop: <rect x="4" y="4" width="8" height="8" rx="1" />,
  trash: <path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8h5.8l.6-8" />,
  terminal: <path d="M2.5 4l4 4-4 4M8 12h5.5" />,
  logs: <path d="M3 4h10M3 8h10M3 12h6" />,
};
function DIcon(props: { name: string }) {
  return (
    <Show when={GLYPHS[props.name]} fallback={<Icon name={props.name} size={14} />}>
      <svg class="svg-icon" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        {GLYPHS[props.name]}
      </svg>
    </Show>
  );
}

// ---- data, refreshed by one poll that only runs while the view is mounted and the window is visible ----
const [status, setStatus] = createSignal<DockerStatus | null>(null);
const [containers, setContainers] = createSignal<DockerContainer[]>([]);
const [compose, setCompose] = createSignal<DockerCompose | null>(null);
const [stats, setStats] = createSignal<Record<string, DockerStat>>({});
const [images, setImages] = createSignal<DockerImage[]>([]);
const [volumes, setVolumes] = createSignal<DockerVolume[]>([]);
const [networks, setNetworks] = createSignal<DockerNetwork[]>([]);
const [loadErr, setLoadErr] = createSignal("");
const [loaded, setLoaded] = createSignal(false);

/** results go to the app toast, like everywhere else */
const say = (text: string, err = false) => (err ? setError(text) : showNotice(text));
const nameOf = (e: unknown) => String(e).replace(/^Error: /, "");

/** shell sessions started from the detail pane, per container id, so coming back to the tab finds the same shell */
const shells = new Map<string, number>();

export function DockerView() {
  const [nav, setNav] = createSignal<Nav>("containers");
  const [q, setQ] = createSignal("");
  const [filter, setFilter] = createSignal<Filter>("all");
  const [open, setOpen] = createSignal<Set<string>>(new Set()); // collapsed compose projects
  const [selId, setSelId] = createSignal<string | null>(null);
  const [curId, setCurId] = createSignal<string | null>(null);
  const [detailTab, setDetailTab] = createSignal<DTab>("info");
  const [busy, setBusy] = createSignal(false);
  let search!: HTMLInputElement;

  let timer: ReturnType<typeof setTimeout> | undefined;
  let polling = false;
  let alive = true;
  const poll = async () => {
    if (polling) return;
    polling = true;
    try {
      const st = await dockerStatus();
      setStatus(st);
      if (st.running) {
        const tab = nav() === "images" ? dockerImages().then(setImages) : nav() === "volumes" ? dockerVolumes().then(setVolumes) : nav() === "networks" ? dockerNetworks().then(setNetworks) : null;
        const c = await dockerContainers(selectedWorktree());
        await tab;
        setContainers((prev) => keepSame(prev, c.containers, (x) => x.id));
        setCompose(c.compose);
        if (c.containers.some(isRunning)) {
          dockerStats().then((l) => alive && setStats(Object.fromEntries(l.map((s) => [s.name, s]))), () => {});
        } else setStats({});
      }
      setLoadErr("");
    } catch (e) {
      setLoadErr(nameOf(e));
    } finally {
      polling = false;
      setLoaded(true);
    }
  };
  // 3 s while docker answers, 10 s while it does not; nothing while the window is hidden
  const loop = async () => {
    if (!document.hidden) await poll();
    if (alive) timer = setTimeout(loop, status()?.running ? FAST_MS : SLOW_MS);
  };
  onMount(() => void loop());
  onCleanup(() => {
    alive = false;
    clearTimeout(timer);
  });
  const pollNow = () => {
    clearTimeout(timer);
    void loop();
  };
  createEffect(on([nav, selectedWorktree], () => status()?.running && void poll(), { defer: true }));

  const selected = createMemo(() => containers().find((c) => c.id === selId()) ?? null);
  // the selected container went away (removed): close its pane
  createEffect(() => {
    if (loaded() && selId() && !selected() && status()?.running) setSelId(null);
  });

  const shown = createMemo(() => filterContainers(containers(), q(), filter()));
  const groupNames = createMemo(() => groupByProject(shown()).map((g) => g.project), [], { equals: (a, b) => a.join("\0") === b.join("\0") });
  const itemsOf = (project: string) => shown().filter((c) => c.project === project).sort((a, b) => a.name.localeCompare(b.name));
  const visibleIds = createMemo(() => groupNames().flatMap((g) => (open().has(g) ? [] : itemsOf(g).map((c) => c.id))));

  // ---- actions ----
  const act = async (done: string, f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
      say(done);
    } catch (e) {
      say(nameOf(e), true);
    } finally {
      setBusy(false);
      pollNow();
    }
  };
  const doAction = (a: DockerAction, c: DockerContainer, done: string, o: { force?: boolean; volumes?: boolean } = {}) =>
    act(`${done}: ${c.name}`, () => dockerAction(a, { id: c.name, ...o }));

  const confirmRemove = async (c: DockerContainer, mode: "plain" | "force" | "volumes") => {
    const what = {
      plain: `Remover o contentor "${c.name}"? Os ficheiros que só existem dentro dele perdem-se. Os volumes ficam.`,
      force: `Remover à força "${c.name}"? O contentor é morto sem aviso (SIGKILL) e apagado. Os ficheiros que só existem dentro dele perdem-se.`,
      volumes: `Remover "${c.name}" e os seus volumes anónimos? Os dados guardados nesses volumes perdem-se. Volumes com nome ficam.`,
    }[mode];
    if (!(await ask(what, { title: mode === "force" ? "Remover à força" : "Remover contentor", kind: "danger", okLabel: mode === "force" ? "Remover à força" : "Remover", cancelLabel: "Cancelar" }))) return;
    if (selId() === c.id) setSelId(null);
    await doAction("container.remove", c, "Removido", { force: mode === "force", volumes: mode === "volumes" });
  };
  const confirmKill = async (c: DockerContainer) => {
    if (await ask(`Matar "${c.name}" sem aviso (SIGKILL)? Não tem oportunidade de guardar nada.`, { title: "Matar contentor", kind: "danger", okLabel: "Matar", cancelLabel: "Cancelar" })) await doAction("container.kill", c, "Morto");
  };

  const bulk = async (group: string, items: DockerContainer[], kind: "up" | "stop" | "rm") => {
    const label = group || "sem projeto";
    let todo = items;
    if (kind === "up") todo = items.filter((c) => !isRunning(c));
    if (kind === "stop") todo = items.filter(isRunning);
    if (!todo.length) return say(kind === "up" ? `"${label}" já está a correr.` : `"${label}" não tem nada a correr.`);
    if (kind === "stop" && !(await ask(`Parar ${todo.length} contentor(es) de "${label}"?\n${todo.map((c) => c.name).join("\n")}`, { title: "Parar grupo", kind: "warning", okLabel: "Parar", cancelLabel: "Cancelar" }))) return;
    if (kind === "rm" && !(await ask(`Apagar ${todo.length} contentor(es) de "${label}", mesmo os que estão a correr?\n${todo.map((c) => c.name).join("\n")}\n\nOs ficheiros que só existem dentro deles perdem-se. Os volumes ficam.`, { title: "Apagar grupo", kind: "danger", okLabel: "Apagar", cancelLabel: "Cancelar" }))) return;
    const action: DockerAction = kind === "up" ? "container.start" : kind === "stop" ? "container.stop" : "container.remove";
    await act(`${kind === "up" ? "Iniciados" : kind === "stop" ? "Parados" : "Apagados"} ${todo.length} em "${label}"`, async () => {
      const failed: string[] = [];
      for (const c of todo) {
        try {
          await dockerAction(action, { id: c.name, force: kind === "rm" });
        } catch (e) {
          failed.push(`${c.name}: ${nameOf(e)}`);
        }
      }
      if (failed.length) throw new Error(failed.join("; "));
    });
  };

  const composeDo = async (kind: "up" | "down") => {
    const wt = selectedWorktree();
    if (!wt) return;
    if (kind === "down" && !(await ask(`Parar e remover os contentores e redes do compose deste worktree (${compose()?.project})? Os volumes ficam.`, { title: "Compose down", kind: "warning", okLabel: "Parar e remover", cancelLabel: "Cancelar" }))) return;
    await act(kind === "up" ? "Compose iniciado" : "Compose parado", () => dockerAction(kind === "up" ? "compose.up" : "compose.down", { worktree: wt }));
  };

  const prune = (kind: "containers" | "images" | "volumes") => {
    const n = kind === "containers" ? containers().filter((c) => !isRunning(c)).length : kind === "images" ? images().filter((i) => i.dangling).length : volumes().filter((v) => !v.used_by.length).length;
    const text = {
      containers: `Remover todos os contentores parados${nav() === "containers" ? ` (${n} agora)` : ""}? Os ficheiros que só existem dentro deles perdem-se. Os volumes ficam.`,
      images: "Remover as imagens sem nome nem etiqueta (dangling), que nenhum contentor usa? Imagens com nome ficam.",
      volumes: "Remover os volumes anónimos que nenhum contentor usa? Os dados guardados neles perdem-se. Volumes com nome ficam.",
    }[kind];
    return ask(text, { title: "Limpar", kind: "danger", okLabel: "Limpar", cancelLabel: "Cancelar" }).then((ok) => {
      if (!ok) return;
      void act("Limpeza feita.", async () => {
        const out = await dockerAction(`prune.${kind}` as DockerAction);
        const last = out.split("\n").filter(Boolean).pop();
        if (last) say(last);
        return out;
      });
    });
  };
  const pruneMenu = (e: MouseEvent) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    showMenuAt(r.left, r.top - 100, [
      { label: "Remover contentores parados…", icon: "trash", run: () => prune("containers") },
      { label: "Remover imagens sem nome…", icon: "trash", run: () => prune("images") },
      { label: "Remover volumes anónimos sem uso…", icon: "trash", run: () => prune("volumes") },
    ]);
  };

  // ---- terminal tabs of the worktree ----
  const openAsTab = async (kind: "logs" | "shell", c: DockerContainer) => {
    const wt = selectedWorktree();
    if (!wt) return say("Escolhe um worktree para abrir o separador.", true);
    try {
      const project = findWorktree(wt)?.project.id;
      const id = await (kind === "logs" ? dockerOpenLogs(wt, c.name, project) : dockerOpenShell(wt, c.name, project));
      await refreshSessions();
      setShowDocker(false);
      openSession(wt, id);
    } catch (e) {
      say(nameOf(e), true);
    }
  };

  const showPane = (c: DockerContainer, tab: DTab) => {
    setSelId(c.id);
    setCurId(c.id);
    setDetailTab(tab);
  };

  const menuFor = (c: DockerContainer): MenuItem[] => [
    ...(isRunning(c)
      ? [
          { label: "Parar", icon: "stop", run: () => doAction("container.stop", c, "Parado") },
          { label: "Reiniciar", icon: "refresh", run: () => doAction("container.restart", c, "Reiniciado") },
          { label: "Pausar", icon: "pause", run: () => doAction("container.pause", c, "Em pausa") },
        ]
      : [{ label: c.state === "paused" ? "Retomar" : "Iniciar", icon: "play", run: () => (c.state === "paused" ? doAction("container.unpause", c, "Retomado") : doAction("container.start", c, "Iniciado")) }]),
    { separator: true },
    { label: "Logs", icon: "logs", run: () => showPane(c, "logs") },
    { label: "Shell", icon: "terminal", disabled: !isRunning(c), run: () => showPane(c, "terminal") },
    { label: "Abrir logs num separador", icon: "external", run: () => openAsTab("logs", c) },
    { label: "Abrir shell num separador", icon: "external", disabled: !isRunning(c), run: () => openAsTab("shell", c) },
    { separator: true },
    { label: "Matar…", icon: "x", danger: true, disabled: !isRunning(c), run: () => confirmKill(c) },
    { label: "Remover…", icon: "trash", danger: true, run: () => confirmRemove(c, "plain") },
    { label: "Remover com volumes anónimos…", icon: "trash", danger: true, run: () => confirmRemove(c, "volumes") },
    { label: "Remover à força…", icon: "trash", danger: true, run: () => confirmRemove(c, "force") },
  ];

  // ---- keyboard ----
  const onListKey = (e: KeyboardEvent) => {
    const t = e.target as HTMLElement;
    if (t.closest("input, textarea") || (t.closest("button") && e.key !== "ArrowDown" && e.key !== "ArrowUp")) return;
    const ids = visibleIds();
    const i = ids.indexOf(curId() ?? "");
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = ids[Math.min(ids.length - 1, Math.max(0, i + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) {
        setCurId(next);
        queueMicrotask(() => document.querySelector<HTMLElement>(`[data-cid="${next}"]`)?.focus());
      }
      return;
    }
    const c = containers().find((x) => x.id === curId());
    if (!c) return;
    if (isMenuKey(e)) {
      e.preventDefault();
      const r = (document.querySelector<HTMLElement>(`[data-cid="${c.id}"]`) ?? (t as HTMLElement)).getBoundingClientRect();
      showMenuAt(r.left + 24, r.bottom, menuFor(c));
    } else if (e.key === "Enter") {
      e.preventDefault();
      showPane(c, "logs");
    } else if (e.key === "Delete") {
      e.preventDefault();
      void confirmRemove(c, "plain");
    }
  };
  const onRootKey = (e: KeyboardEvent) => {
    const t = e.target as HTMLElement;
    if (e.key === "/" && !t.closest("input, textarea, .xterm")) {
      e.preventDefault();
      search.focus();
    } else if (e.key === "Escape" && !t.closest(".xterm")) {
      if (q() && t === search) setQ("");
      else if (selId()) setSelId(null);
    }
  };

  const retry = () => {
    setLoaded(false);
    pollNow();
  };

  return (
    <div class="dk" role="region" aria-label="Docker" onKeyDown={onRootKey}>
      <nav class="dk-nav" aria-label="Docker">
        <For each={NAV}>
          {([id, label]) => (
            <Button variant="ghost" class={cn("dk-nav-item h-auto justify-between px-2.5 py-1.5 font-normal text-muted-foreground", nav() === id && "on bg-secondary text-foreground")} aria-current={nav() === id ? "page" : undefined} onClick={() => { setNav(id); setQ(""); }}>
              <span>{label}</span>
              <span class="dk-count">{id === "containers" ? containers().length || "" : id === "volumes" ? volumes().length || "" : id === "images" ? images().length || "" : networks().length || ""}</span>
            </Button>
          )}
        </For>
        <span class="grow" />
        <Show when={status()?.running}>
          <Button variant="ghost" class="dk-nav-item h-auto justify-between px-2.5 py-1.5 font-normal text-muted-foreground" onClick={pruneMenu} aria-haspopup="menu">
            <span>Limpar…</span>
          </Button>
          <div class="auto-note dk-ver">Docker {status()?.version}</div>
        </Show>
      </nav>

      <section class="dk-main" aria-label="Lista">
        <Show
          when={status()?.running}
          fallback={<Unavailable status={status()} err={loadErr()} loaded={loaded()} retry={retry} />}
        >
          <div class="dk-tools">
            <Input
              ref={search}
              class="dk-search min-w-0 flex-1"
              type="search"
              placeholder="Pesquisar (/)"
              aria-label="Pesquisar"
              value={q()}
              onInput={(e) => setQ(e.currentTarget.value)}
            />
            <Show when={nav() === "containers"}>
              <div class="seg dk-seg" role="group" aria-label="Filtrar por estado">
                <For each={[["all", "Todos"], ["running", "A correr"], ["stopped", "Parados"]] as [Filter, string][]}>
                  {([f, label]) => (
                    <Button size="sm" class={cn(filter() === f && "on border-primary bg-popover ring-1 ring-primary")} aria-pressed={filter() === f} onClick={() => setFilter(f)}>
                      {label}
                    </Button>
                  )}
                </For>
              </div>
            </Show>
            <Button variant="icon" size="icon" aria-label="Atualizar" title="Atualizar" onClick={retry}>
              <Icon name="refresh" size={14} />
            </Button>
          </div>
          <Show when={loadErr()}>
            <div class="dk-err" role="alert">{loadErr()}</div>
          </Show>

          <div class="dk-body">
            <Show when={nav() === "containers"}>
              <ComposeBar compose={compose()} wt={selectedWorktree()} busy={busy()} run={composeDo} />
              <Show when={shown().length} fallback={<Empty text={containers().length ? "Nenhum contentor corresponde à pesquisa ou ao filtro." : "Ainda não há contentores. Crie um com docker run ou docker compose up e aparece aqui."} />}>
                <div class="dk-list" role="listbox" aria-label="Contentores" tabindex="0" onKeyDown={onListKey}>
                  <For each={groupNames()}>
                    {(g) => {
                      const items = createMemo(() => itemsOf(g));
                      const collapsed = () => open().has(g);
                      const running = () => items().filter(isRunning).length;
                      const toggle = () => setOpen((s) => { const n = new Set(s); n.has(g) ? n.delete(g) : n.add(g); return n; });
                      return (
                        <div class="dk-group">
                          <div class="dk-ghead">
                            <Button variant="ghost" class="dk-gtoggle h-auto px-0 py-0.5 font-normal hover:bg-transparent" aria-expanded={!collapsed()} onClick={toggle}>
                              <span class="dk-chev" classList={{ shut: collapsed() }}><Icon name="chevDown" size={12} /></span>
                              <strong>{g || "Sem projeto"}</strong>
                              <span class="auto-note">{running()}/{items().length} a correr</span>
                            </Button>
                            <span class="grow" />
                            <Button variant="ghost" size="sm" class="dk-gbtn text-muted-foreground" disabled={busy()} onClick={() => bulk(g, items(), "up")}>Iniciar</Button>
                            <Button variant="ghost" size="sm" class="dk-gbtn text-muted-foreground" disabled={busy()} onClick={() => bulk(g, items(), "stop")}>Parar</Button>
                            <Button variant="ghost" size="sm" class="dk-gbtn danger text-muted-foreground hover:text-destructive" disabled={busy()} onClick={() => bulk(g, items(), "rm")}>Apagar</Button>
                          </div>
                          <Show when={!collapsed()}>
                            <For each={items()}>
                              {(c) => (
                                <Row
                                  c={c}
                                  stat={stats()[c.name]}
                                  sel={selId() === c.id}
                                  cur={curId() === c.id}
                                  busy={busy()}
                                  onPick={() => showPane(c, detailTab())}
                                  onMenu={(e) => { setCurId(c.id); showMenu(e, menuFor(c)); }}
                                  onStart={() => doAction("container.start", c, "Iniciado")}
                                  onStop={() => doAction("container.stop", c, "Parado")}
                                  onRestart={() => doAction("container.restart", c, "Reiniciado")}
                                  onLogs={() => showPane(c, "logs")}
                                  onShell={() => showPane(c, "terminal")}
                                  onRemove={() => confirmRemove(c, "plain")}
                                />
                              )}
                            </For>
                          </Show>
                        </div>
                      );
                    }}
                  </For>
                </div>
              </Show>
            </Show>

            <Show when={nav() === "volumes"}>
              <VolumesList list={volumes()} q={q()} busy={busy()} onRemove={async (v) => {
                if (await ask(`Remover o volume "${v.name}"? Os dados guardados nele perdem-se.`, { title: "Remover volume", kind: "danger", okLabel: "Remover", cancelLabel: "Cancelar" }))
                  await act(`Volume removido: ${v.name}`, () => dockerAction("volume.remove", { id: v.name }));
              }} />
            </Show>
            <Show when={nav() === "images"}>
              <ImagesList list={images()} q={q()} busy={busy()} onRemove={async (i) => {
                if (await ask(`Remover a imagem ${i.tags[0] ?? i.id}? Pode ser descarregada ou construída de novo.`, { title: "Remover imagem", kind: "danger", okLabel: "Remover", cancelLabel: "Cancelar" }))
                  await act(`Imagem removida: ${i.tags[0] ?? i.id}`, () => dockerAction("image.remove", { id: i.id }));
              }} />
            </Show>
            <Show when={nav() === "networks"}>
              <NetworksList list={networks()} q={q()} />
            </Show>
          </div>
        </Show>
      </section>

      <Show when={nav() === "containers" && selId()} keyed>
        {(id) => (
          <Show when={selected()} fallback={null}>
            <Detail
              id={id}
              tab={detailTab()}
              setTab={setDetailTab}
              c={selected()!}
              stat={stats()[selected()!.name]}
              onClose={() => setSelId(null)}
              onOpenTab={openAsTab}
            />
          </Show>
        )}
      </Show>
    </div>
  );
}

function Empty(props: { text: string }) {
  return <div class="dk-empty auto-note">{props.text}</div>;
}

function Unavailable(props: { status: DockerStatus | null; err: string; loaded: boolean; retry: () => void }) {
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

function ComposeBar(props: { compose: DockerCompose | null; wt: string | null; busy: boolean; run: (k: "up" | "down") => void }) {
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

function Row(props: {
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

function VolumesList(props: { list: DockerVolume[]; q: string; busy: boolean; onRemove: (v: DockerVolume) => void }) {
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

function ImagesList(props: { list: DockerImage[]; q: string; busy: boolean; onRemove: (i: DockerImage) => void }) {
  const rows = () => props.list.filter((i) => has(props.q, i.id, ...i.tags));
  return (
    <Show when={rows().length} fallback={<Empty text={props.list.length ? "Nenhuma imagem corresponde à pesquisa." : "Não há imagens."} />}>
      <div class="dk-list">
        <For each={rows()}>
          {(i) => {
            const users = () => imageUsers(i, containers());
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

function NetworksList(props: { list: DockerNetwork[]; q: string }) {
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

// ---- detail pane ----
function Detail(props: { id: string; c: DockerContainer; stat?: DockerStat; tab: DTab; setTab: (t: DTab) => void; onClose: () => void; onOpenTab: (k: "logs" | "shell", c: DockerContainer) => void }) {
  const run = () => isRunning(props.c);
  const tabs: HTMLButtonElement[] = [];
  const onTabKey = (e: KeyboardEvent) => {
    const i = DTABS.findIndex(([t]) => t === props.tab);
    const n = e.key === "ArrowRight" ? i + 1 : e.key === "ArrowLeft" ? i - 1 : e.key === "Home" ? 0 : e.key === "End" ? DTABS.length - 1 : null;
    if (n == null) return;
    e.preventDefault();
    const k = (n + DTABS.length) % DTABS.length;
    props.setTab(DTABS[k][0]);
    tabs[k]?.focus();
  };
  return (
    <aside class="dk-detail" aria-label={`Detalhes de ${props.c.name}`}>
      <div class="dk-dhead">
        <span class="dot" classList={{ on: stateInfo(props.c.state).cls === "on", warn: stateInfo(props.c.state).cls === "warn" }} />
        <strong class="dk-dtitle" title={props.c.name}>{props.c.name}</strong>
        <span class="auto-note">{stateInfo(props.c.state).text}</span>
        <span class="grow" />
        <Button variant="icon" size="icon" aria-label="Fechar detalhes" title="Fechar (Esc)" onClick={props.onClose}><Icon name="x" size={14} /></Button>
      </div>
      <div class="dk-dtabs" role="tablist" aria-label="Detalhes" onKeyDown={onTabKey}>
        <For each={DTABS}>
          {([t, label], i) => (
            <Button variant="ghost" ref={(el) => (tabs[i()] = el)} role="tab" aria-selected={props.tab === t} tabindex={props.tab === t ? 0 : -1} class={cn("h-auto rounded-none border-0 border-b-2 px-2.5 py-1.5 font-normal text-muted-foreground hover:bg-transparent hover:text-foreground", props.tab === t ? "on border-primary text-foreground" : "border-transparent")} onClick={() => props.setTab(t)}>
              {label}
            </Button>
          )}
        </For>
      </div>
      <div class="dk-dbody" role="tabpanel">
        <Show when={props.tab === "info"}><InfoTab c={props.c} stat={props.stat} /></Show>
        <Show when={props.tab === "logs"}><LogsTab c={props.c} onOpenTab={() => props.onOpenTab("logs", props.c)} /></Show>
        <Show when={props.tab === "terminal"}><ShellTab c={props.c} run={run()} onOpenTab={() => props.onOpenTab("shell", props.c)} /></Show>
        <Show when={props.tab === "files"}><FilesTab c={props.c} run={run()} /></Show>
      </div>
    </aside>
  );
}

function InfoTab(props: { c: DockerContainer; stat?: DockerStat }) {
  const [info, setInfo] = createSignal<DockerInspect | null>(null);
  const [err, setErr] = createSignal("");
  const [shown, setShown] = createSignal<Set<string>>(new Set());
  createEffect(on(() => props.c.id, () => dockerInspect(props.c.name).then((i) => { setInfo(i); setErr(""); }, (e) => setErr(nameOf(e)))));
  const chips = () => portChips(props.c.ports);
  return (
    <div class="dk-info">
      <Show when={err()}><div class="dk-err" role="alert">{err()}</div></Show>
      <dl>
        <dt>Imagem</dt><dd>{props.c.image}</dd>
        <dt>Estado</dt><dd>{props.c.status}</dd>
        <dt>ID</dt><dd class="mono">{props.c.id.slice(0, 12)}</dd>
        <dt>Criado</dt><dd>{props.c.created}</dd>
        <Show when={props.c.project}><dt>Compose</dt><dd>{props.c.project}{props.c.service ? ` · ${props.c.service}` : ""}</dd></Show>
        <Show when={props.stat}><dt>CPU / memória</dt><dd>{props.stat!.cpu.toFixed(1)}% · {props.stat!.mem}</dd></Show>
        <Show when={info()}>
          <dt>Comando</dt><dd class="mono">{info()!.command || "—"}</dd>
          <Show when={info()!.workdir}><dt>Pasta</dt><dd class="mono">{info()!.workdir}</dd></Show>
          <Show when={info()!.restart}><dt>Reinício</dt><dd>{info()!.restart}</dd></Show>
        </Show>
      </dl>
      <Show when={chips().length}>
        <h4>Portas</h4>
        <div class="dk-ports">
          <For each={chips()}>{(p) => (p.url ? <Button variant="outline" size="bare" class="px-1.5 text-xs" aria-label={`Abrir ${p.url} no navegador`} onClick={() => void openUrl(p.url!)}>{p.label}</Button> : <Badge>{p.label}</Badge>)}</For>
        </div>
      </Show>
      <Show when={info()}>
        <h4>Redes</h4>
        <Show when={info()!.networks.length} fallback={<span class="auto-note">Sem redes.</span>}>
          <ul class="dk-kv"><For each={info()!.networks}>{([n, ip]) => <li><span>{n}</span><span class="mono">{ip}</span></li>}</For></ul>
        </Show>
        <h4>Montagens</h4>
        <Show when={info()!.mounts.length} fallback={<span class="auto-note">Sem montagens.</span>}>
          <ul class="dk-kv"><For each={info()!.mounts}>{([kind, src, dst, rw]) => <li><span class="mono" title={src}>{src}</span><span class="mono">{dst} <span class="auto-note">{kind}{rw ? "" : " · só leitura"}</span></span></li>}</For></ul>
        </Show>
        <h4>Variáveis de ambiente</h4>
        <Show when={info()!.env.length} fallback={<span class="auto-note">Nenhuma.</span>}>
          <ul class="dk-kv">
            <For each={info()!.env}>
              {([k, v]) => {
                const secret = isSecretKey(k);
                const visible = () => !secret || shown().has(k);
                return (
                  <li>
                    <span class="mono">{k}</span>
                    <span class="mono dk-val">
                      {visible() ? v : "••••••••"}
                      <Show when={secret}>
                        <Button variant="link" size="bare" class="text-primary underline hover:text-primary" aria-label={`${visible() ? "Esconder" : "Mostrar"} ${k}`} onClick={() => setShown((s) => { const n = new Set(s); n.has(k) ? n.delete(k) : n.add(k); return n; })}>
                          {visible() ? "esconder" : "mostrar"}
                        </Button>
                      </Show>
                    </span>
                  </li>
                );
              }}
            </For>
          </ul>
        </Show>
      </Show>
    </div>
  );
}

function LogsTab(props: { c: DockerContainer; onOpenTab: () => void }) {
  const [text, setText] = createSignal("");
  const [err, setErr] = createSignal("");
  const [follow, setFollow] = createSignal(true);
  const [query, setQuery] = createSignal("");
  let pre!: HTMLPreElement;
  const load = () => dockerLogs(props.c.name, 500).then((t) => { setText(t); setErr(""); }, (e) => setErr(nameOf(e)));
  void load();
  const timer = setInterval(() => follow() && !document.hidden && void load(), FAST_MS);
  onCleanup(() => clearInterval(timer));
  const lines = createMemo(() => searchLog(text(), query()));
  createEffect(() => {
    lines();
    if (follow() && pre) queueMicrotask(() => (pre.scrollTop = pre.scrollHeight));
  });
  return (
    <div class="dk-logs">
      <div class="dk-tools">
        <Input class="dk-search min-w-0 flex-1" type="search" placeholder="Procurar nos logs" aria-label="Procurar nos logs" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} />
        <Checkbox labelClass="set-check" checked={follow()} onChange={(e) => setFollow(e.currentTarget.checked)} label="Seguir" />
        <Button size="sm" onClick={props.onOpenTab}>Abrir num separador</Button>
      </div>
      <Show when={err()}><div class="dk-err" role="alert">{err()}</div></Show>
      <pre ref={pre} class="dk-pre" tabindex="0" aria-label="Logs">{lines().join("\n") || (text() ? "Nada corresponde à pesquisa." : "Sem logs.")}</pre>
      <div class="auto-note">{query() ? `${lines().length} linhas correspondem · ` : ""}últimas 500 linhas, atualizadas a cada 3 s</div>
    </div>
  );
}

function ShellTab(props: { c: DockerContainer; run: boolean; onOpenTab: () => void }) {
  const [sid, setSid] = createSignal<number | null>(shells.get(props.c.id) ?? null);
  const [err, setErr] = createSignal("");
  const live = () => {
    const id = sid();
    return id != null && sessions().some((s) => s.id === id && s.running) ? id : null;
  };
  const start = async () => {
    const wt = selectedWorktree();
    if (!wt) return setErr("Escolhe um worktree para abrir a shell.");
    try {
      const id = await dockerOpenShell(wt, props.c.name, findWorktree(wt)?.project.id);
      shells.set(props.c.id, id);
      await refreshSessions();
      setSid(id);
      setErr("");
    } catch (e) {
      setErr(nameOf(e));
    }
  };
  return (
    <div class="dk-term">
      <Show when={live() != null} fallback={
        <div class="dk-empty">
          <span class="auto-note">{props.run ? "Abre uma shell dentro do contentor (bash se existir, senão sh)." : "O contentor está parado: inicie-o para abrir uma shell."}</span>
          <Button variant="default" size="sm" class="primary" disabled={!props.run} onClick={() => void start()}>Abrir shell</Button>
          <Show when={err()}><div class="dk-err" role="alert">{err()}</div></Show>
        </div>
      }>
        <div class="dk-tools"><span class="grow" /><Button size="sm" onClick={props.onOpenTab}>Abrir num separador</Button></div>
        <div class="dk-termhost"><TerminalView id={live()!} focus onExit={() => { shells.delete(props.c.id); setSid(null); void refreshSessions(); }} /></div>
      </Show>
    </div>
  );
}

function FilesTab(props: { c: DockerContainer; run: boolean }) {
  const [path, setPath] = createSignal("/");
  const [entries, setEntries] = createSignal<DockerFsEntry[]>([]);
  const [file, setFile] = createSignal<{ path: string; text: string } | null>(null);
  const [err, setErr] = createSignal("");
  createEffect(() => {
    if (!props.run) return;
    const p = path();
    dockerFsList(props.c.name, p).then((l) => { setEntries(l); setErr(""); }, (e) => { setEntries([]); setErr(nameOf(e)); });
  });
  const pick = async (e: DockerFsEntry) => {
    const full = joinPath(path(), e.name);
    if (e.dir) {
      setFile(null);
      return setPath(full);
    }
    try {
      setFile({ path: full, text: await dockerFsRead(props.c.name, full) });
      setErr("");
    } catch (x) {
      setFile(null);
      setErr(nameOf(x));
    }
  };
  return (
    <Show when={props.run} fallback={<Empty text="O contentor está parado: os ficheiros só se veem enquanto corre." />}>
      <div class="dk-files">
        <div class="dk-tools">
          <Button variant="icon" size="icon" aria-label="Pasta acima" title="Pasta acima" disabled={path() === "/"} onClick={() => { setFile(null); setPath(joinPath(path(), "..")); }}><Icon name="arrowUp" size={14} /></Button>
          <code class="dk-code grow">{path()}</code>
        </div>
        <Show when={err()}><div class="dk-err" role="alert">{err()}</div></Show>
        <ul class="dk-flist">
          <For each={entries()}>
            {(e) => (
              <li>
                <Button variant="ghost" size="sm" class={cn("h-auto w-full justify-start px-2 py-0.5 font-normal", file()?.path === joinPath(path(), e.name) && "on bg-secondary")} onClick={() => void pick(e)}>
                  <Icon name={e.dir ? "folder" : "file"} size={13} /> {e.name}
                </Button>
              </li>
            )}
          </For>
        </ul>
        <Show when={file()}>
          {(f) => (
            <>
              <div class="auto-note mono">{f().path} · só leitura</div>
              <pre class="dk-pre" tabindex="0" aria-label={`Conteúdo de ${f().path}`}>{f().text}</pre>
            </>
          )}
        </Show>
      </div>
    </Show>
  );
}
