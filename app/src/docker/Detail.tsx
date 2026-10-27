import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, untrack } from "solid-js";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  DockerContainer,
  DockerFsEntry,
  DockerInspect,
  DockerStat,
  dockerFsList,
  dockerFsRead,
  dockerInspect,
  dockerLogs,
  dockerOpenShell,
} from "../api";
import { createLive } from "../live";
import { Icon } from "../Icon";
import { Badge, Button, Checkbox, Input, cn } from "../ui";
import { TerminalView } from "../TerminalView";
import { createLatest, isRunning, isSecretKey, joinPath, portChips, searchLog, stateInfo } from "../dockerLogic";
import { findWorktree, refreshSessions, selectedWorktree, sessions } from "../store";
import { nameOf, shells, DTab, DTABS } from "./shared";
import { Empty } from "./Lists";
import { P } from "../strings.panels";
import { fmtFixed } from "../i18n";

// ---- detail pane ----
export function Detail(props: {
  id: string;
  c: DockerContainer;
  stat?: DockerStat;
  tab: DTab;
  setTab: (t: DTab) => void;
  onClose: () => void;
  onOpenTab: (k: "logs" | "shell", c: DockerContainer) => void;
}) {
  const run = () => isRunning(props.c);
  const tabs: HTMLButtonElement[] = [];
  const onTabKey = (e: KeyboardEvent) => {
    const i = DTABS.findIndex(([t]) => t === props.tab);
    const n =
      e.key === "ArrowRight"
        ? i + 1
        : e.key === "ArrowLeft"
          ? i - 1
          : e.key === "Home"
            ? 0
            : e.key === "End"
              ? DTABS.length - 1
              : null;
    if (n == null) return;
    e.preventDefault();
    const k = (n + DTABS.length) % DTABS.length;
    props.setTab(DTABS[k][0]);
    tabs[k]?.focus();
  };
  return (
    <aside class="dk-detail" aria-label={P.docker.detailsOf(props.c.name)}>
      <div class="dk-dhead">
        <span
          class="dot"
          classList={{ on: stateInfo(props.c.state).cls === "on", warn: stateInfo(props.c.state).cls === "warn" }}
        />
        <strong class="dk-dtitle" title={props.c.name}>
          {props.c.name}
        </strong>
        <span class="auto-note">{stateInfo(props.c.state).text}</span>
        <span class="grow" />
        <Button
          variant="icon"
          size="icon"
          aria-label={P.docker.closeDetails}
          title={P.docker.closeDetailsTip}
          onClick={props.onClose}
        >
          <Icon name="x" size={14} />
        </Button>
      </div>
      <div class="dk-dtabs" role="tablist" aria-label={P.docker.details} onKeyDown={onTabKey}>
        <For each={DTABS}>
          {([t, label], i) => (
            <Button
              variant="ghost"
              ref={(el) => (tabs[i()] = el)}
              role="tab"
              aria-selected={props.tab === t}
              tabindex={props.tab === t ? 0 : -1}
              class={cn(
                "h-auto rounded-none border-0 border-b-2 px-2.5 py-1.5 font-normal text-muted-foreground hover:bg-transparent hover:text-foreground",
                props.tab === t ? "on border-primary text-foreground" : "border-transparent",
              )}
              onClick={() => props.setTab(t)}
            >
              {label}
            </Button>
          )}
        </For>
      </div>
      <div class="dk-dbody" role="tabpanel">
        <Show when={props.tab === "info"}>
          <InfoTab c={props.c} stat={props.stat} />
        </Show>
        <Show when={props.tab === "logs"}>
          <LogsTab c={props.c} onOpenTab={() => props.onOpenTab("logs", props.c)} />
        </Show>
        <Show when={props.tab === "terminal"}>
          <ShellTab c={props.c} run={run()} onOpenTab={() => props.onOpenTab("shell", props.c)} />
        </Show>
        <Show when={props.tab === "files"}>
          <FilesTab c={props.c} run={run()} />
        </Show>
      </div>
    </aside>
  );
}

export function InfoTab(props: { c: DockerContainer; stat?: DockerStat }) {
  const [info, setInfo] = createSignal<DockerInspect | null>(null);
  const [err, setErr] = createSignal("");
  const [shown, setShown] = createSignal<Set<string>>(new Set());
  createEffect(
    on(
      () => props.c.id,
      () =>
        dockerInspect(props.c.name).then(
          (i) => {
            setInfo(i);
            setErr("");
          },
          (e) => setErr(nameOf(e)),
        ),
    ),
  );
  const chips = () => portChips(props.c.ports);
  return (
    <div class="dk-info">
      <Show when={err()}>
        <div class="dk-err" role="alert">
          {err()}
        </div>
      </Show>
      <dl>
        <dt>{P.docker.info.image}</dt>
        <dd>{props.c.image}</dd>
        <dt>{P.docker.info.state}</dt>
        <dd>{props.c.status}</dd>
        <dt>ID</dt>
        <dd class="mono">{props.c.id.slice(0, 12)}</dd>
        <dt>{P.docker.info.created}</dt>
        <dd>{props.c.created}</dd>
        <Show when={props.c.project}>
          <dt>Compose</dt>
          <dd>
            {props.c.project}
            {props.c.service ? ` · ${props.c.service}` : ""}
          </dd>
        </Show>
        <Show when={props.stat}>
          <dt>{P.docker.info.cpuMem}</dt>
          <dd>
            {fmtFixed(props.stat!.cpu, 1)}% · {props.stat!.mem}
          </dd>
        </Show>
        <Show when={info()}>
          <dt>{P.docker.info.command}</dt>
          <dd class="mono">{info()!.command || "—"}</dd>
          <Show when={info()!.workdir}>
            <dt>{P.docker.info.folder}</dt>
            <dd class="mono">{info()!.workdir}</dd>
          </Show>
          <Show when={info()!.restart}>
            <dt>{P.docker.info.restart}</dt>
            <dd>{info()!.restart}</dd>
          </Show>
        </Show>
      </dl>
      <Show when={chips().length}>
        <h4>{P.docker.info.ports}</h4>
        <div class="dk-ports">
          <For each={chips()}>
            {(p) =>
              p.url ? (
                <Button
                  variant="outline"
                  size="bare"
                  class="px-1.5 text-xs"
                  aria-label={P.docker.openUrl(p.url)}
                  onClick={() => void openUrl(p.url!)}
                >
                  {p.label}
                </Button>
              ) : (
                <Badge>{p.label}</Badge>
              )
            }
          </For>
        </div>
      </Show>
      <Show when={info()}>
        <h4>{P.docker.info.networks}</h4>
        <Show when={info()!.networks.length} fallback={<span class="auto-note">{P.docker.info.noNetworks}</span>}>
          <ul class="dk-kv">
            <For each={info()!.networks}>
              {([n, ip]) => (
                <li>
                  <span>{n}</span>
                  <span class="mono">{ip}</span>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <h4>{P.docker.info.mounts}</h4>
        <Show when={info()!.mounts.length} fallback={<span class="auto-note">{P.docker.info.noMounts}</span>}>
          <ul class="dk-kv">
            <For each={info()!.mounts}>
              {([kind, src, dst, rw]) => (
                <li>
                  <span class="mono" title={src}>
                    {src}
                  </span>
                  <span class="mono">
                    {dst}{" "}
                    <span class="auto-note">
                      {kind}
                      {rw ? "" : P.docker.info.readOnly}
                    </span>
                  </span>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <h4>{P.docker.info.env}</h4>
        <Show when={info()!.env.length} fallback={<span class="auto-note">{P.docker.info.none}</span>}>
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
                        <Button
                          variant="link"
                          size="bare"
                          class="text-primary underline hover:text-primary"
                          aria-label={visible() ? P.docker.info.hideKey(k) : P.docker.info.showKey(k)}
                          onClick={() =>
                            setShown((s) => {
                              const n = new Set(s);
                              n.has(k) ? n.delete(k) : n.add(k);
                              return n;
                            })
                          }
                        >
                          {visible() ? P.docker.info.hide : P.docker.info.show}
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

export function LogsTab(props: { c: DockerContainer; onOpenTab: () => void }) {
  const [text, setText] = createSignal("");
  const [err, setErr] = createSignal("");
  const [follow, setFollow] = createSignal(true);
  const [query, setQuery] = createSignal("");
  let pre!: HTMLPreElement;
  // one request at a time (`createLive` runs the next load after the one in flight) and a token so an answer that arrives after
  // the component moved on never overwrites a newer one
  const latest = createLatest();
  let gone = false;
  const load = () => {
    const t = latest.next();
    return dockerLogs(props.c.name, 500).then(
      (r) => {
        if (!gone && latest.isCurrent(t)) {
          setText(r);
          setErr("");
        }
      },
      (e) => {
        if (!gone && latest.isCurrent(t)) setErr(nameOf(e));
      },
    );
  };
  const poller = createLive(() => follow() && load(), ["docker.changed"]);
  poller.now();
  onCleanup(() => {
    gone = true;
    poller.stop();
  });
  const lines = createMemo(() => searchLog(text(), query()));
  createEffect(() => {
    lines();
    if (follow() && pre) queueMicrotask(() => (pre.scrollTop = pre.scrollHeight));
  });
  return (
    <div class="dk-logs">
      <div class="dk-tools">
        <Input
          class="dk-search min-w-0 flex-1"
          type="search"
          placeholder={P.docker.logsSearch}
          aria-label={P.docker.logsSearch}
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
        />
        <Checkbox
          labelClass="set-check"
          checked={follow()}
          onChange={(e) => setFollow(e.currentTarget.checked)}
          label={P.docker.follow}
        />
        <Button size="sm" onClick={props.onOpenTab}>
          {P.docker.openInTab}
        </Button>
      </div>
      <Show when={err()}>
        <div class="dk-err" role="alert">
          {err()}
        </div>
      </Show>
      <pre ref={pre} class="dk-pre" tabindex="0" aria-label={P.docker.menu.logs}>
        {lines().join("\n") || (text() ? P.docker.logsNoMatch : P.docker.noLogs)}
      </pre>
      <div class="auto-note">
        {query() ? P.docker.logsLinesMatch(lines().length) : ""}
        {P.docker.logsFooter}
      </div>
    </div>
  );
}

export function ShellTab(props: { c: DockerContainer; run: boolean; onOpenTab: () => void }) {
  const [sid, setSid] = createSignal<number | null>(untrack(() => shells.get(props.c.id) ?? null));
  const [err, setErr] = createSignal("");
  const live = () => {
    const id = sid();
    return id != null && sessions().some((s) => s.id === id && s.running) ? id : null;
  };
  const start = async () => {
    const wt = selectedWorktree();
    if (!wt) return setErr(P.docker.pickWorktreeShell);
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
      <Show
        when={live() != null}
        fallback={
          <div class="dk-empty">
            <span class="auto-note">{props.run ? P.docker.shellHintRun : P.docker.shellHintStopped}</span>
            <Button variant="default" size="sm" class="primary" disabled={!props.run} onClick={() => void start()}>
              {P.docker.openShell}
            </Button>
            <Show when={err()}>
              <div class="dk-err" role="alert">
                {err()}
              </div>
            </Show>
          </div>
        }
      >
        <div class="dk-tools">
          <span class="grow" />
          <Button size="sm" onClick={props.onOpenTab}>
            {P.docker.openInTab}
          </Button>
        </div>
        <div class="dk-termhost">
          <TerminalView
            id={live()!}
            focus
            onExit={() => {
              shells.delete(props.c.id);
              setSid(null);
              void refreshSessions();
            }}
          />
        </div>
      </Show>
    </div>
  );
}

export function FilesTab(props: { c: DockerContainer; run: boolean }) {
  const [path, setPath] = createSignal("/");
  const [entries, setEntries] = createSignal<DockerFsEntry[]>([]);
  const [file, setFile] = createSignal<{ path: string; text: string } | null>(null);
  const [err, setErr] = createSignal("");
  createEffect(() => {
    if (!props.run) return;
    const p = path();
    dockerFsList(props.c.name, p).then(
      (l) => {
        setEntries(l);
        setErr("");
      },
      (e) => {
        setEntries([]);
        setErr(nameOf(e));
      },
    );
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
    <Show when={props.run} fallback={<Empty text={P.docker.filesStopped} />}>
      <div class="dk-files">
        <div class="dk-tools">
          <Button
            variant="icon"
            size="icon"
            aria-label={P.docker.parentFolder}
            title={P.docker.parentFolder}
            disabled={path() === "/"}
            onClick={() => {
              setFile(null);
              setPath(joinPath(path(), ".."));
            }}
          >
            <Icon name="arrowUp" size={14} />
          </Button>
          <code class="dk-code grow">{path()}</code>
        </div>
        <Show when={err()}>
          <div class="dk-err" role="alert">
            {err()}
          </div>
        </Show>
        <ul class="dk-flist">
          <For each={entries()}>
            {(e) => (
              <li>
                <Button
                  variant="ghost"
                  size="sm"
                  class={cn(
                    "h-auto w-full justify-start px-2 py-0.5 font-normal",
                    file()?.path === joinPath(path(), e.name) && "on bg-secondary",
                  )}
                  onClick={() => void pick(e)}
                >
                  <Icon name={e.dir ? "folder" : "file"} size={13} /> {e.name}
                </Button>
              </li>
            )}
          </For>
        </ul>
        <Show when={file()}>
          {(f) => (
            <>
              <div class="auto-note mono">{P.docker.readOnlyNote(f().path)}</div>
              <pre class="dk-pre" tabindex="0" aria-label={P.docker.contentOf(f().path)}>
                {f().text}
              </pre>
            </>
          )}
        </Show>
      </div>
    </Show>
  );
}
