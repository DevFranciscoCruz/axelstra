import { For, Show, createEffect, createMemo, createSignal, onMount } from "solid-js";
import { AddHostForm } from "./AddHostForm";
import { HostBadge, reconnectWithNotice } from "./HostBadge";
import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { BrowseResult, hostBrowse, hostInfo, projectAddRemote } from "./api";
import { errorText } from "./appError";
import { breadcrumbs, classifyRemoteError, joinRemote, moveRow, pathProblem, sepOf } from "./hostLogic";
import {
  hostById,
  hosts,
  projects,
  refreshProjects,
  refreshSessions,
  selectWorktree,
  setError,
  showNotice,
} from "./store";
import { S } from "./strings";
import { R } from "./strings.remote";
import { AP } from "./strings.sources";
import { Badge, Button, Field, Input, Select } from "./ui";

const NEW = "__new__";
type Failure = { text: string; detail: string };

/** Pick a host (or add one, with the WSL distros and ssh hosts found on this machine as suggestions), type or browse the folder on it, confirm. The Tauri host asks its own native confirmation and checks the folder exists. `embedded` (inside the Add project dialog) drops the form's own title bar, the dialog has one. */
export function AddRemoteForm(props: { onClose: () => void; embedded?: boolean }) {
  const [host, setHost] = createSignal(hosts().find((h) => h.state === "ok")?.id ?? hosts()[0]?.id ?? "");
  const [adding, setAdding] = createSignal(hosts().length === 0);
  const [path, setPath] = createSignal("");
  const [listing, setListing] = createSignal<BrowseResult | null>(null);
  const [row, setRow] = createSignal(-1);
  const [loading, setLoading] = createSignal(false);
  const [failure, setFailure] = createSignal<Failure | null>(null);
  const [pathError, setPathError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [prefill, setPrefill] = createSignal<{ name: string; spec: string; n: number } | null>(null);
  const [found, setFound] = createSignal<{ spec: string; name: string; kind: "wsl" | "ssh" }[]>([]);
  let seq = 0;
  let pathInput: HTMLInputElement | undefined;

  const info = () => hostById(host());
  const sep = () => sepOf(listing()?.path ?? path());
  const crumbs = createMemo(() => breadcrumbs(listing()?.path ?? ""));
  const dirs = () => listing()?.entries ?? [];

  const fail = (e: unknown) => {
    const detail = errorText(e);
    setFailure({ text: R.failure[classifyRemoteError(detail)], detail });
  };

  /** list `at` (the home directory when omitted) on the chosen host; a stale answer is dropped */
  const browse = async (at?: string) => {
    const h = host();
    if (!h) return;
    const mine = ++seq;
    setLoading(true);
    setFailure(null);
    try {
      const r = await hostBrowse(h, at);
      if (mine !== seq) return;
      setListing(r);
      setPath(r.path);
      setRow(-1);
      setPathError("");
    } catch (e) {
      if (mine === seq) {
        setListing(null);
        fail(e);
      }
    } finally {
      if (mine === seq) setLoading(false);
    }
  };

  // WSL distros and ssh hosts this machine knows: suggestions for a new host (an older daemon lacks the fields, none shown)
  onMount(() => {
    void hostInfo("local")
      .then((i) =>
        setFound([
          ...(i?.wsl_distros ?? []).map((d) => ({ spec: `wsl:${d}`, name: d, kind: "wsl" as const })),
          ...(i?.ssh_hosts ?? []).map((h) => ({ spec: `ssh:${h}`, name: h, kind: "ssh" as const })),
        ]),
      )
      .catch((e) => console.debug("host.info", e));
  });
  const suggestions = (kind: "wsl" | "ssh") =>
    found().filter((f) => f.kind === kind && !hosts().some((h) => h.spec === f.spec));

  // choosing another host starts again at its home directory
  createEffect(() => {
    const h = host();
    setListing(null);
    setPath("");
    setFailure(null);
    if (h && !adding()) void browse();
  });

  const open = (name: string) => {
    const l = listing();
    if (l) void browse(joinRemote(l.path, name, sep()));
  };

  const reconnect = async () => {
    await reconnectWithNotice(host());
    void browse(path() || undefined);
  };

  const submit = async () => {
    const p = path().trim();
    const bad = pathProblem(p);
    if (bad) {
      setPathError(bad === "empty" ? R.pathEmpty : R.pathRelative);
      pathInput?.focus();
      return;
    }
    if (!host() || busy()) return;
    setBusy(true);
    setFailure(null);
    try {
      const added = await projectAddRemote(host(), p);
      await refreshProjects();
      await refreshSessions();
      const main = projects()
        .find((x) => x.id === added.id)
        ?.worktrees.find((w) => w.is_main);
      if (main) selectWorktree(main.path);
      showNotice(R.projectAdded(added.name, info()?.name ?? host()));
      props.onClose();
    } catch (e) {
      const msg = errorText(e);
      // the native confirmation was declined
      if (/cancel|recus|declin|negad/i.test(msg)) showNotice(R.cancelled);
      else if (classifyRemoteError(msg) === "other") setError(msg);
      else fail(e);
    } finally {
      setBusy(false);
    }
  };

  const onListKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && row() >= 0) {
      e.preventDefault();
      open(dirs()[row()].name);
    } else if (e.key === "Backspace" && listing()?.parent) {
      e.preventDefault();
      void browse(listing()!.parent!);
    } else if (["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) {
      e.preventDefault();
      setRow(moveRow(row(), dirs().length, e.key));
      document.getElementById(`rb-row-${row()}`)?.scrollIntoView?.({ block: "nearest" });
    }
  };

  return (
    <>
      <Show when={!props.embedded}>
        <div class="auto-head">
          <strong>{R.addRemoteTitle}</strong>
          <span class="grow" />
          <Button onClick={props.onClose}>{S.close}</Button>
        </div>
      </Show>

      <Field label={R.host}>
        <Select
          label={R.host}
          value={adding() ? NEW : host()}
          options={[...hosts().map((h) => ({ value: h.id, label: h.name })), { value: NEW, label: R.newHost }]}
          onChange={(v) => {
            if (v === NEW) setAdding(true);
            else {
              setAdding(false);
              setHost(v);
            }
          }}
        />
      </Field>

      <Show when={found().length > 0 && (suggestions("wsl").length > 0 || suggestions("ssh").length > 0)}>
        <div class="flex flex-col gap-1.5">
          <For each={["wsl", "ssh"] as const}>
            {(kind) => (
              <Show when={suggestions(kind).length > 0}>
                <div
                  role="group"
                  aria-label={kind === "wsl" ? AP.suggestWsl : AP.suggestSsh}
                  class="flex flex-wrap items-center gap-1.5"
                >
                  <span class="text-xs text-muted-foreground">{kind === "wsl" ? AP.suggestWsl : AP.suggestSsh}</span>
                  <For each={suggestions(kind)}>
                    {(f) => (
                      <Button
                        type="button"
                        size="sm"
                        title={AP.suggestUse(f.spec)}
                        aria-label={AP.suggestUse(f.spec)}
                        onClick={() => {
                          setAdding(true);
                          setPrefill({ name: f.name, spec: f.spec, n: Date.now() });
                        }}
                      >
                        {f.name}
                      </Button>
                    )}
                  </For>
                </div>
              </Show>
            )}
          </For>
          <span class="auto-note">{AP.suggestNote}</span>
        </div>
      </Show>

      <Show when={adding()}>
        <AddHostForm
          prefill={prefill()}
          onAdded={(id) => {
            setHost(id);
            setAdding(false);
          }}
          onCancel={hosts().length ? () => setAdding(false) : undefined}
        />
      </Show>

      <Show when={!adding() && host()}>
        <Show when={info()}>{(h) => <HostBadge host={h().id} />}</Show>
        <Show when={failure()}>
          {(f) => (
            <div class="flex flex-col gap-1 rounded-md border border-[var(--danger)] bg-[var(--err-bg)] p-2" role="alert">
              <span>{f().text}</span>
              <details class="text-xs text-muted-foreground">
                <summary>{R.detail}</summary>
                <pre class="m-0 whitespace-pre-wrap break-words">{f().detail}</pre>
              </details>
              <div class="m-actions">
                <Button type="button" variant="default" size="sm" onClick={() => void reconnect()}>
                  {R.reconnect}
                </Button>
              </div>
            </div>
          )}
        </Show>

        <Field label={R.path} hint={R.pathHint} full>
          <Input
            ref={pathInput}
            value={path()}
            placeholder={R.pathPh}
            autocomplete="off"
            spellcheck={false}
            aria-invalid={!!pathError()}
            aria-describedby="rb-path-error"
            onInput={(e) => {
              setPath(e.currentTarget.value);
              setPathError("");
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                // Enter browses a typed path; with the list showing that same folder it confirms
                if (listing()?.path === path().trim()) void submit();
                else if (!pathProblem(path())) void browse(path().trim());
                else setPathError(path().trim() ? R.pathRelative : R.pathEmpty);
              } else if (e.key === "ArrowDown" && dirs().length) {
                e.preventDefault();
                setRow(0);
                document.getElementById("rb-list")?.focus();
              }
            }}
          />
        </Field>
        <div id="rb-path-error" class="auto-note text-[var(--danger)]" role="alert">
          {pathError()}
        </div>

        <Show when={listing()}>
          {(l) => (
            <>
              <nav aria-label={R.crumbs} class="flex flex-wrap items-center gap-1 text-xs">
                <Button
                  type="button"
                  variant="icon"
                  size="icon"
                  class="size-6"
                  title={R.up}
                  aria-label={R.up}
                  disabled={!l().parent}
                  onClick={() => l().parent && void browse(l().parent!)}
                >
                  <span aria-hidden="true">↑</span>
                </Button>
                <Button
                  type="button"
                  variant="link"
                  size="bare"
                  title={R.home}
                  aria-label={R.home}
                  onClick={() => void browse(l().home)}
                >
                  <Icon name="folder" size={12} />
                </Button>
                <For each={crumbs()}>
                  {(c, i) => (
                    <>
                      <Show when={i() > 0}>
                        <span aria-hidden="true">{sep()}</span>
                      </Show>
                      <Button
                        type="button"
                        variant="link"
                        size="bare"
                        aria-current={i() === crumbs().length - 1 ? "location" : undefined}
                        onClick={() => void browse(c.path)}
                      >
                        {c.label}
                      </Button>
                    </>
                  )}
                </For>
              </nav>
              <Show
                when={dirs().length > 0}
                fallback={<div class="auto-note">{loading() ? R.browsing : R.emptyDir}</div>}
              >
                <ul
                  id="rb-list"
                  role="listbox"
                  tabindex="0"
                  aria-label={`${R.browseLabel} ${l().path} (${R.browseCount(dirs().length)})`}
                  aria-activedescendant={row() >= 0 ? `rb-row-${row()}` : undefined}
                  class="m-0 max-h-60 list-none overflow-auto rounded-md border border-border p-1 outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
                  onKeyDown={onListKey}
                >
                  <For each={dirs()}>
                    {(d, i) => (
                      <li
                        id={`rb-row-${i()}`}
                        role="option"
                        aria-selected={row() === i()}
                        class="flex cursor-pointer items-center gap-2 rounded-sm px-2 py-1 text-base hover:bg-accent aria-selected:bg-accent"
                        onClick={() => setRow(i())}
                        onDblClick={() => open(d.name)}
                      >
                        <Icon name="folder" size={13} />
                        <span class="truncate">{d.name}</span>
                        <Show when={d.git}>
                          <Badge title={R.gitTip}>git</Badge>
                        </Show>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
              <Show when={l().truncated}>
                <div class="auto-note">{R.truncated}</div>
              </Show>
            </>
          )}
        </Show>
        <Show when={loading() && !listing()}>
          <div class="auto-note" role="status">
            {R.browsing}
          </div>
        </Show>

        <div class="m-actions">
          <Button type="button" onClick={props.onClose}>
            {S.cancel}
          </Button>
          <Button type="button" variant="default" disabled={busy() || !path().trim()} onClick={() => void submit()}>
            {busy() ? R.confirming : R.confirm}
          </Button>
        </div>
      </Show>
    </>
  );
}

/** The remote form on its own dialog (used by tests and anywhere the form is wanted without the choice step). */
export function AddRemoteProject(props: { onClose: () => void }) {
  return (
    <Modal label={R.addRemoteTitle} class="modal" onClose={props.onClose}>
      <AddRemoteForm onClose={props.onClose} />
    </Modal>
  );
}
