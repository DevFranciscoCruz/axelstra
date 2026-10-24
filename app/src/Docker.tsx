import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { DockerAction, DockerCompose, DockerContainer, DockerImage, DockerNetwork, DockerStat, DockerStatus, DockerVolume, dockerAction, dockerContainers, dockerImages, dockerNetworks, dockerOpenLogs, dockerOpenShell, dockerStats, dockerStatus, dockerVolumes } from "./api";
import { ask } from "./Confirm";
import { MenuItem, isMenuKey, showMenu, showMenuAt } from "./ContextMenu";
import { Icon } from "./Icon";
import { Button, Input, cn } from "./ui";
import { Filter, pruneShells, filterContainers, groupByProject, isRunning } from "./dockerLogic";
import { findWorktree, keepSame, openSession, refreshSessions, selectedWorktree, sessions, setShowDocker, showDocker } from "./store";
import { say, nameOf, shells, DTab } from "./docker/shared";
import { Empty, Unavailable, ComposeBar, Row, VolumesList, ImagesList, NetworksList } from "./docker/Lists";
import { FAST_MS } from "./docker/shared";
import { Detail } from "./docker/Detail";
import "./docker.css";

/** The Docker view fills the centre area like the Kanban; whoever mounts it owns this signal. */
export { showDocker, setShowDocker };

type Nav = "containers" | "volumes" | "images" | "networks";
const NAV: [Nav, string][] = [["containers", "Contentores"], ["volumes", "Volumes"], ["images", "Imagens"], ["networks", "Redes"]];
const SLOW_MS = 10000;


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




export function DockerView() {
  createEffect(() => pruneShells(shells, sessions().filter((s) => s.running).map((s) => s.id)));
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
              <ImagesList list={images()} containers={containers()} q={q()} busy={busy()} onRemove={async (i) => {
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
