import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { DockerAction, DockerCompose, DockerContainer, DockerImage, DockerNetwork, DockerStat, DockerStatus, DockerVolume, dockerAction, dockerContainers, dockerImages, dockerNetworks, dockerOpenLogs, dockerOpenShell, dockerStats, dockerStatus, dockerVolumes } from "./api";
import { ask } from "./Confirm";
import { MenuItem, isMenuKey, showMenu, showMenuAt } from "./ContextMenu";
import { Icon } from "./Icon";
import { Button, Input, cn } from "./ui";
import { Filter, pruneShells, filterContainers, groupByProject, isRunning } from "./dockerLogic";
import { findWorktree, keepSame, navStamp, openIfStill, refreshSessions, selectedWorktree, sessions, setShowDocker, showDocker } from "./store";
import { say, nameOf, shells, DTab } from "./docker/shared";
import { Empty, Unavailable, ComposeBar, Row, VolumesList, ImagesList, NetworksList } from "./docker/Lists";
import { Detail } from "./docker/Detail";
import { createLive } from "./live";
import { P } from "./strings.panels";
import "./docker.css";

/** The Docker view fills the centre area like the Kanban; whoever mounts it owns this signal. */
export { showDocker, setShowDocker };

type Nav = "containers" | "volumes" | "images" | "networks";
const NAV: [Nav, string][] = (["containers", "volumes", "images", "networks"] as Nav[]).map((n) => [n, P.docker.nav[n]]);


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

  let alive = true;
  const poll = async () => {
    if (document.hidden) return;
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
      setLoaded(true);
    }
  };
  // the daemon says when docker changed (`docker.changed`: containers, images, volumes, networks, docker going up or down);
  // the window showing again also looks once. `createLive` runs one load at a time.
  const live = createLive(poll, ["docker.changed"]);
  onMount(() => live.now());
  onCleanup(() => {
    alive = false;
    live.stop();
  });
  const pollNow = () => live.now();
  createEffect(on([nav, selectedWorktree], () => status()?.running && live.now(), { defer: true }));

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

  const removeVolume = async (v: DockerVolume) => {
    if (await ask(P.docker.removeVolumeAsk(v.name), { title: P.docker.removeVolumeTitle, kind: "danger", okLabel: P.docker.remove, cancelLabel: P.docker.cancel }))
      await act(P.docker.volumeRemoved(v.name), () => dockerAction("volume.remove", { id: v.name }));
  };
  const removeImage = async (i: DockerImage) => {
    if (await ask(P.docker.removeImageAsk(i.tags[0] ?? i.id), { title: P.docker.removeImageTitle, kind: "danger", okLabel: P.docker.remove, cancelLabel: P.docker.cancel }))
      await act(P.docker.imageRemoved(i.tags[0] ?? i.id), () => dockerAction("image.remove", { id: i.id }));
  };

  const confirmRemove = async (c: DockerContainer, mode: "plain" | "force" | "volumes") => {
    const what = {
      plain: P.docker.removeAsk.plain(c.name),
      force: P.docker.removeAsk.force(c.name),
      volumes: P.docker.removeAsk.volumes(c.name),
    }[mode];
    if (!(await ask(what, { title: mode === "force" ? P.docker.forceTitle : P.docker.removeTitle, kind: "danger", okLabel: mode === "force" ? P.docker.forceTitle : P.docker.remove, cancelLabel: P.docker.cancel }))) return;
    if (selId() === c.id) setSelId(null);
    await doAction("container.remove", c, P.docker.done.removed, { force: mode === "force", volumes: mode === "volumes" });
  };
  const confirmKill = async (c: DockerContainer) => {
    if (await ask(P.docker.killAsk(c.name), { title: P.docker.killTitle, kind: "danger", okLabel: P.docker.kill, cancelLabel: P.docker.cancel })) await doAction("container.kill", c, P.docker.done.killed);
  };

  const bulk = async (group: string, items: DockerContainer[], kind: "up" | "stop" | "rm") => {
    const label = group || P.docker.noProjectLower;
    let todo = items;
    if (kind === "up") todo = items.filter((c) => !isRunning(c));
    if (kind === "stop") todo = items.filter(isRunning);
    if (!todo.length) return say(kind === "up" ? P.docker.alreadyRunning(label) : P.docker.nothingRunning(label));
    if (kind === "stop" && !(await ask(P.docker.stopGroupAsk(todo.length, label, todo.map((c) => c.name).join("\n")), { title: P.docker.stopGroupTitle, kind: "warning", okLabel: P.docker.stop, cancelLabel: P.docker.cancel }))) return;
    if (kind === "rm" && !(await ask(P.docker.deleteGroupAsk(todo.length, label, todo.map((c) => c.name).join("\n")), { title: P.docker.deleteGroupTitle, kind: "danger", okLabel: P.docker.delete, cancelLabel: P.docker.cancel }))) return;
    const action: DockerAction = kind === "up" ? "container.start" : kind === "stop" ? "container.stop" : "container.remove";
    await act(P.docker.groupDone(kind, todo.length, label), async () => {
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
    if (kind === "down" && !(await ask(P.docker.composeDownAsk(compose()?.project), { title: P.docker.composeDownTitle, kind: "warning", okLabel: P.docker.composeDownOk, cancelLabel: P.docker.cancel }))) return;
    await act(kind === "up" ? P.docker.composeUp : P.docker.composeDown, () => dockerAction(kind === "up" ? "compose.up" : "compose.down", { worktree: wt }));
  };

  const prune = (kind: "containers" | "images" | "volumes") => {
    const n = kind === "containers" ? containers().filter((c) => !isRunning(c)).length : kind === "images" ? images().filter((i) => i.dangling).length : volumes().filter((v) => !v.used_by.length).length;
    const text = {
      containers: P.docker.pruneAsk.containers(nav() === "containers" ? n : null),
      images: P.docker.pruneAsk.images,
      volumes: P.docker.pruneAsk.volumes,
    }[kind];
    return ask(text, { title: P.docker.pruneTitle, kind: "danger", okLabel: P.docker.pruneTitle, cancelLabel: P.docker.cancel }).then((ok) => {
      if (!ok) return;
      void act(P.docker.pruneDone, async () => {
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
      { label: P.docker.pruneMenu.containers, icon: "trash", run: () => prune("containers") },
      { label: P.docker.pruneMenu.images, icon: "trash", run: () => prune("images") },
      { label: P.docker.pruneMenu.volumes, icon: "trash", run: () => prune("volumes") },
    ]);
  };

  // ---- terminal tabs of the worktree ----
  const openAsTab = async (kind: "logs" | "shell", c: DockerContainer) => {
    const wt = selectedWorktree();
    if (!wt) return say(P.docker.pickWorktreeTab, true);
    const stamp = navStamp();
    try {
      const project = findWorktree(wt)?.project.id;
      const id = await (kind === "logs" ? dockerOpenLogs(wt, c.name, project) : dockerOpenShell(wt, c.name, project));
      await refreshSessions();
      setShowDocker(false);
      openIfStill(stamp, wt, id);
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
          { label: P.docker.menu.stop, icon: "stop", run: () => doAction("container.stop", c, P.docker.done.stopped) },
          { label: P.docker.menu.restart, icon: "refresh", run: () => doAction("container.restart", c, P.docker.done.restarted) },
          { label: P.docker.menu.pause, icon: "pause", run: () => doAction("container.pause", c, P.docker.done.paused) },
        ]
      : [{ label: c.state === "paused" ? P.docker.menu.resume : P.docker.menu.start, icon: "play", run: () => (c.state === "paused" ? doAction("container.unpause", c, P.docker.done.resumed) : doAction("container.start", c, P.docker.done.started)) }]),
    { separator: true },
    { label: P.docker.menu.logs, icon: "logs", run: () => showPane(c, "logs") },
    { label: P.docker.menu.shell, icon: "terminal", disabled: !isRunning(c), run: () => showPane(c, "terminal") },
    { label: P.docker.menu.logsTab, icon: "external", run: () => openAsTab("logs", c) },
    { label: P.docker.menu.shellTab, icon: "external", disabled: !isRunning(c), run: () => openAsTab("shell", c) },
    { separator: true },
    { label: P.docker.menu.kill, icon: "x", danger: true, disabled: !isRunning(c), run: () => confirmKill(c) },
    { label: P.docker.menu.remove, icon: "trash", danger: true, run: () => confirmRemove(c, "plain") },
    { label: P.docker.menu.removeVolumes, icon: "trash", danger: true, run: () => confirmRemove(c, "volumes") },
    { label: P.docker.menu.removeForce, icon: "trash", danger: true, run: () => confirmRemove(c, "force") },
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
    <div class="dk" role="region" aria-label={P.docker.region} onKeyDown={onRootKey}>
      <nav class="dk-nav" aria-label={P.docker.region}>
        <For each={NAV}>
          {([id, label]) => (
            <Button variant="ghost" class={cn("dk-nav-item h-auto justify-between px-2.5 py-1.5 font-normal text-muted-foreground", nav() === id && "on bg-secondary text-foreground")} aria-current={nav() === id ? "page" : undefined} onClick={() => { setNav(id); setQ(""); }}>
              <span>{label}</span>
              <span class="dk-count">{id === "containers" ? containers().length || "" : id === "volumes" ? volumes().length || "" : id === "images" ? images().length || "" : networks().length || ""}</span>
            </Button>
          )}
        </For>
        <span class="grow" role="gridcell" />
        <Show when={status()?.running}>
          <Button variant="ghost" class="dk-nav-item h-auto justify-between px-2.5 py-1.5 font-normal text-muted-foreground" onClick={pruneMenu} aria-haspopup="menu">
            <span>{P.docker.prune}</span>
          </Button>
          <div class="auto-note dk-ver">Docker {status()?.version}</div>
        </Show>
      </nav>

      <section class="dk-main" aria-label={P.docker.list}>
        <Show
          when={status()?.running}
          fallback={<Unavailable status={status()} err={loadErr()} loaded={loaded()} retry={retry} />}
        >
          <div class="dk-tools">
            <Input
              ref={search}
              class="dk-search min-w-0 flex-1"
              type="search"
              placeholder={P.docker.searchPlaceholder}
              aria-label={P.docker.search}
              value={q()}
              onInput={(e) => setQ(e.currentTarget.value)}
            />
            <Show when={nav() === "containers"}>
              <div class="seg dk-seg" role="group" aria-label={P.docker.filterBy}>
                <For each={(["all", "running", "stopped"] as Filter[]).map((f) => [f, P.docker.filters[f]] as [Filter, string])}>
                  {([f, label]) => (
                    <Button size="sm" class={cn(filter() === f && "on border-primary bg-popover ring-1 ring-primary")} aria-pressed={filter() === f} onClick={() => setFilter(f)}>
                      {label}
                    </Button>
                  )}
                </For>
              </div>
            </Show>
            <Button variant="icon" size="icon" aria-label={P.docker.refresh} title={P.docker.refresh} onClick={retry}>
              <Icon name="refresh" size={14} />
            </Button>
          </div>
          <Show when={loadErr()}>
            <div class="dk-err" role="alert">{loadErr()}</div>
          </Show>

          <div class="dk-body">
            <Show when={nav() === "containers"}>
              <ComposeBar compose={compose()} wt={selectedWorktree()} busy={busy()} run={composeDo} />
              <Show when={shown().length} fallback={<Empty text={containers().length ? P.docker.empty.noMatch : P.docker.empty.none} />}>
                <div class="dk-list" role="grid" aria-label={P.docker.nav.containers} tabindex="0" onKeyDown={onListKey}>
                  <For each={groupNames()}>
                    {(g) => {
                      const items = createMemo(() => itemsOf(g));
                      const collapsed = () => open().has(g);
                      const running = () => items().filter(isRunning).length;
                      const toggle = () => setOpen((s) => { const n = new Set(s); n.has(g) ? n.delete(g) : n.add(g); return n; });
                      return (
                        <div class="dk-group" role="rowgroup">
                          <div class="dk-ghead" role="row">
                            <span role="gridcell" class="contents"><Button variant="ghost" class="dk-gtoggle h-auto px-0 py-0.5 font-normal hover:bg-transparent" aria-expanded={!collapsed()} onClick={toggle}>
                              <span class="dk-chev" classList={{ shut: collapsed() }}><Icon name="chevDown" size={12} /></span>
                              <strong>{g || P.docker.noProject}</strong>
                              <span class="auto-note">{P.docker.runningOf(running(), items().length)}</span>
                            </Button></span>
                            <span class="grow" role="gridcell" />
                            <span role="gridcell" class="contents"><Button variant="ghost" size="sm" class="dk-gbtn text-muted-foreground" disabled={busy()} onClick={() => bulk(g, items(), "up")}>{P.docker.start}</Button></span>
                            <span role="gridcell" class="contents"><Button variant="ghost" size="sm" class="dk-gbtn text-muted-foreground" disabled={busy()} onClick={() => bulk(g, items(), "stop")}>{P.docker.stop}</Button></span>
                            <span role="gridcell" class="contents"><Button variant="ghost" size="sm" class="dk-gbtn danger text-muted-foreground hover:text-destructive" disabled={busy()} onClick={() => bulk(g, items(), "rm")}>{P.docker.delete}</Button></span>
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
                                  onStart={() => doAction("container.start", c, P.docker.done.started)}
                                  onStop={() => doAction("container.stop", c, P.docker.done.stopped)}
                                  onRestart={() => doAction("container.restart", c, P.docker.done.restarted)}
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
              <VolumesList list={volumes()} q={q()} busy={busy()} onRemove={removeVolume} />
            </Show>
            <Show when={nav() === "images"}>
              <ImagesList list={images()} containers={containers()} q={q()} busy={busy()} onRemove={removeImage} />
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
