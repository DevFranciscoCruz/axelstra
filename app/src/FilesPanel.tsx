import { Loading } from "./Loading";
import { ask } from "./Confirm";
import { Button, Input, Tabs, TabsList, TabsTrigger } from "./ui";
import { Icon } from "./Icon";
import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { For, Show, createEffect, createMemo, createResource, createSignal, on, onCleanup } from "solid-js";
import { PAGE, capList, sameListing } from "./listing";
import { press } from "./a11y";
import { FsEntry, GrepHit, fsCreate, fsDelete, fsGrep, fsList, fsRename, fsSearch } from "./api";
import { MenuItem, menuKey, showMenu } from "./ContextMenu";
import { RightTabs } from "./RightTabs";
import { changes, findWorktree, fsTick, guarded, openFile, selectedWorktree, setError, setOpenCommit, setOpenDiff, setOpenFile, setShowKanban, rightTab, showNotice } from "./store";
import { S } from "./strings";
import { P } from "./strings.panels";

/** labels of the two "new" entries, kept when a menu is trimmed to them */
const NEW_ITEMS: string[] = [P.files.newFile, P.files.newFolder];
const join = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);
const parentOf = (p: string) => p.slice(0, Math.max(0, p.lastIndexOf("/")));
const baseOf = (p: string) => p.slice(p.lastIndexOf("/") + 1);

/** folders left open; "" is the worktree root */
const [expanded, setExpanded] = createSignal<ReadonlySet<string>>(new Set([""]));
const isOpen = (p: string) => expanded().has(p);
const toggle = (p: string) =>
  setExpanded((cur) => {
    const n = new Set(cur);
    if (!n.delete(p)) n.add(p);
    return n;
  });
const expand = (p: string) => setExpanded((cur) => new Set(cur).add(p));

/** bumped to reload every visible folder from disk */
const [gen, setGen] = createSignal(0);

/** inline name box: a new file/folder inside `parent`, or a rename of `target` */
type Editing = { mode: "file" | "dir" | "rename"; parent: string; target?: string };
const [editing, setEditing] = createSignal<Editing | null>(null);

const openPath = (path: string) =>
  guarded(() => {
    setOpenDiff(null);
    setShowKanban(false);
    setOpenCommit(null);
    setOpenFile(path);
  });

const Folder = (props: { open: boolean }) => (
  <svg class="ficon-svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round">
    <Show when={props.open} fallback={<path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h2.6l1.4 1.5h5A1.5 1.5 0 0 1 14 6v5.5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z" />}>
      <path d="M2 11.5V4.5A1.5 1.5 0 0 1 3.5 3h2.6l1.4 1.5h5A1.5 1.5 0 0 1 14 6v1.2M2.3 12.4l1.3-4.2a1 1 0 0 1 1-.7h9.2a.8.8 0 0 1 .8 1l-1.1 3.9a1.3 1.3 0 0 1-1.3.9H3.6a1.3 1.3 0 0 1-1.3-.9z" />
    </Show>
  </svg>
);
const FileGlyph = () => (
  <svg class="ficon-svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round">
    <path d="M4 2h5l3 3v8.5a.5.5 0 0 1-.5.5h-7.5a.5.5 0 0 1-.5-.5v-11A.5.5 0 0 1 4 2zM9 2v3h3" />
  </svg>
);
const Chevron = (props: { open: boolean }) => (
  <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" class="transition-transform duration-100" style={{ transform: props.open ? "rotate(90deg)" : "none" }}>
    <path d="M4.5 3l3 3-3 3" />
  </svg>
);

/** name box shared by "new" and "rename" */
function NameInput(props: { initial: string; depth: number; dir: boolean; onSubmit: (name: string) => void }) {
  let done = false;
  const finish = (ok: boolean, v: string) => {
    if (done) return;
    done = true;
    if (ok && v.trim() && v.trim() !== props.initial) props.onSubmit(v.trim());
    else setEditing(null);
  };
  return (
    <div class="file editing" style={{ "padding-left": `${8 + props.depth * 12}px` }}>
      <span class="twisty" />
      {props.dir ? <Folder open={false} /> : <FileGlyph />}
      <Input
        class="fname-input"
        value={props.initial}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") finish(true, e.currentTarget.value);
          else if (e.key === "Escape") finish(false, "");
        }}
        onBlur={(e) => finish(true, e.currentTarget.value)}
        ref={(el) =>
          queueMicrotask(() => {
            el.focus();
            const dot = props.initial.lastIndexOf(".");
            el.setSelectionRange(0, dot > 0 ? dot : props.initial.length);
          })
        }
      />
    </div>
  );
}

function Dir(props: { wt: string; path: string; depth: number }) {
  const [entries, setEntries] = createSignal<FsEntry[] | null>(null);
  const [err, setErr] = createSignal("");
  // only the newest listing counts: a slow older answer must not overwrite a newer one
  let req = 0;
  const load = () => {
    const r = ++req;
    void fsList(props.wt, props.path)
      .then((list) => {
        if (r !== req) return;
        setErr("");
        // an unchanged listing keeps its rows (and their open folders) as they are
        if (!sameListing(entries(), list)) setEntries(list);
      })
      .catch((e) => r === req && setErr(String(e)));
  };
  load();
  // the daemon reports file changes: keep the visible folders current. A burst of events asks once, and a hidden window not at all
  // (it asks again when the window is shown).
  let tickTimer: ReturnType<typeof setTimeout> | undefined;
  let stale = false;
  createEffect(
    on(
      [fsTick, gen],
      () => {
        clearTimeout(tickTimer);
        tickTimer = setTimeout(() => (document.hidden ? (stale = true) : load()), 250);
      },
      { defer: true },
    ),
  );
  const onVisible = () => {
    if (!document.hidden && stale) {
      stale = false;
      load();
    }
  };
  document.addEventListener("visibilitychange", onVisible);
  onCleanup(() => {
    clearTimeout(tickTimer);
    document.removeEventListener("visibilitychange", onVisible);
  });
  const [limit, setLimit] = createSignal(PAGE);
  const page = () => capList(entries() ?? [], limit());

  const newHere = () => {
    const e = editing();
    return e && e.mode !== "rename" && e.parent === props.path ? e : null;
  };
  const submitNew = async (name: string) => {
    const e = newHere()!;
    setEditing(null);
    try {
      await fsCreate(props.wt, join(props.path, name), e.mode === "dir");
      setGen(gen() + 1);
      if (e.mode === "file") openPath(join(props.path, name));
    } catch (er) {
      setError(String(er));
    }
  };

  return (
    <>
      <Show when={err()}>
        <div class="empty">{err()}</div>
      </Show>
      <Show when={entries() === null && !err()}>
        <Loading class="empty" />
      </Show>
      <Show when={newHere()}>{(e) => <NameInput initial="" depth={props.depth} dir={e().mode === "dir"} onSubmit={(n) => void submitNew(n)} />}</Show>
      <For each={page().shown}>{(e) => <Node wt={props.wt} dir={props.path} e={e} depth={props.depth} />}</For>
      <Show when={page().hidden > 0}>
        <Button variant="outline" size="sm" class="show-more" style={{ "margin-left": `${8 + props.depth * 12}px` }} onClick={() => setLimit(limit() + PAGE)}>
          {P.files.showMore(page().hidden)}
        </Button>
      </Show>
      <Show when={entries() !== null && entries()!.length === 0 && !newHere()}>
        <div class="empty" style={{ "padding-left": `${8 + props.depth * 12}px` }}>
          {P.files.empty}
        </div>
      </Show>
    </>
  );
}

/** actions shared by the tree rows and the search results */
function fileMenu(wt: string, path: string, dir: boolean): MenuItem[] {
  const abs = `${wt.replace(/[\\/]$/, "")}/${path}`;
  const copy = (text: string) => invoke("clipboard_write", { text }).catch((e) => setError(String(e)));
  const here = dir ? path : parentOf(path);
  return [
    ...(dir ? [] : [{ label: P.files.open, icon: "external", run: () => openPath(path) } as MenuItem]),
    { label: P.files.newFile, icon: "plus", run: () => (expand(here), setEditing({ mode: "file", parent: here })) },
    { label: P.files.newFolder, icon: "folder", run: () => (expand(here), setEditing({ mode: "dir", parent: here })) },
    { separator: true },
    { label: P.files.copyPath, icon: "copy", run: () => copy(abs.replace(/\//g, "\\")) },
    { label: P.files.copyRelative, icon: "copy", run: () => copy(path) },
    { label: P.files.reveal, icon: "folder", run: () => revealItemInDir(abs.replace(/\//g, "\\")).catch((e) => setError(String(e))) },
    { separator: true },
    { label: P.files.rename, icon: "pencil", run: () => setEditing({ mode: "rename", parent: parentOf(path), target: path }) },
    {
      label: P.files.delete,
      icon: "x",
      danger: true,
      run: async () => {
        const ok = await ask(P.files.deleteAsk(baseOf(path), dir), {
          title: P.files.delete,
          kind: "danger",
          okLabel: P.files.delete,
          cancelLabel: P.files.cancel,
        });
        if (!ok) return;
        try {
          await fsDelete(wt, path);
          if (openFile() === path || (dir && openFile()?.startsWith(path + "/"))) setOpenFile(null);
          setGen(gen() + 1);
          showNotice(dir ? S.folderDeleted(baseOf(path)) : S.fileDeleted(baseOf(path)));
        } catch (e) {
          setError(S.deleteFailed(baseOf(path), e));
        }
      },
    },
  ];
}

type Tally = { files?: number; added: number; removed: number };
/** what git says changed: per file, and per folder (every ancestor, "" = the whole tree) */
const changeIndex = createMemo(() => {
  const byFile = new Map<string, { status: string; added: number; removed: number; binary: boolean }>();
  const byDir = new Map<string, Tally>();
  for (const f of changes()?.files ?? []) {
    byFile.set(f.path, f);
    const parts = f.path.split("/");
    for (let i = 0; i < parts.length; i++) {
      const dir = parts.slice(0, i).join("/");
      const t: Required<Tally> = (byDir.get(dir) as Required<Tally> | undefined) ?? { files: 0, added: 0, removed: 0 };
      t.files += 1;
      t.added += f.added;
      t.removed += f.removed;
      byDir.set(dir, t);
    }
  }
  return { byFile, byDir };
});

/** +added −removed, green and red; with a file count for folders */
function Delta(props: { t: Tally | undefined; folder?: boolean; binary?: boolean }) {
  return (
    <Show when={props.t}>
      {(t) => (
        <span class="nums">
          <Show when={props.folder}>
            <span class="fcount" title={P.files.changedFiles(t().files ?? 0)}>{t().files}</span>
          </Show>
          <Show when={props.binary}>
            <span class="fbin">{P.files.bin}</span>
          </Show>
          <Show when={t().added > 0}>
            <span class="add">+{t().added}</span>
          </Show>
          <Show when={t().removed > 0}>
            <span class="del">−{t().removed}</span>
          </Show>
        </span>
      )}
    </Show>
  );
}

function Node(props: { wt: string; dir: string; e: FsEntry; depth: number }) {
  const path = () => join(props.dir, props.e.name);
  const open = () => props.e.dir && isOpen(path());
  const pad = () => ({ "padding-left": `${8 + props.depth * 12}px` });
  const renaming = () => {
    const e = editing();
    return e?.mode === "rename" && e.target === path();
  };
  const submitRename = async (name: string) => {
    setEditing(null);
    try {
      const to = join(props.dir, name);
      await fsRename(props.wt, path(), to);
      if (openFile() === path()) setOpenFile(to);
      setGen(gen() + 1);
    } catch (er) {
      setError(String(er));
    }
  };
  return (
    <>
      <Show
        when={!renaming()}
        fallback={<NameInput initial={props.e.name} depth={props.depth} dir={props.e.dir} onSubmit={(n) => void submitRename(n)} />}
      >
        <div
          class="file"
          classList={{ active: !props.e.dir && openFile() === path() }}
          style={pad()}
          {...press(
            () => (props.e.dir ? toggle(path()) : openPath(path())),
            "button",
            (e) => {
              if (menuKey(() => fileMenu(props.wt, path(), props.e.dir))(e)) return;
              if (e.key === "F2") setEditing({ mode: "rename", parent: props.dir, target: path() });
              else if (e.key === "Delete") showMenu(new MouseEvent("contextmenu", { clientX: 40, clientY: 40 }), fileMenu(props.wt, path(), props.e.dir).slice(-1));
            },
          )}
          aria-expanded={props.e.dir ? open() : undefined}
          onContextMenu={(e) => showMenu(e, fileMenu(props.wt, path(), props.e.dir))}
        >
          <span class="twisty">{props.e.dir ? <Chevron open={open()} /> : ""}</span>
          {props.e.dir ? <Folder open={open()} /> : <FileGlyph />}
          <span class="fname" classList={{ [`ch-${changeIndex().byFile.get(path())?.status}`]: !props.e.dir && changeIndex().byFile.has(path()), "ch-dir": props.e.dir && changeIndex().byDir.has(path()) }}>{props.e.name}</span>
          <Show when={props.e.dir} fallback={<Delta t={changeIndex().byFile.get(path())} binary={changeIndex().byFile.get(path())?.binary} />}>
            <Delta t={changeIndex().byDir.get(path())} folder />
          </Show>
        </div>
      </Show>
      <Show when={open()}>
        <div class="kids" style={{ "--gx": `${8 + props.depth * 12 + 6}px` }}>
          <Dir wt={props.wt} path={path()} depth={props.depth + 1} />
        </div>
      </Show>
    </>
  );
}

export function FilesPanel() {
  const [q, setQ] = createSignal("");
  const [dq, setDq] = createSignal("");
  const [mode, setMode] = createSignal<"names" | "contents">("names");
  let timer: ReturnType<typeof setTimeout> | undefined;
  const onSearch = (v: string) => {
    setQ(v);
    clearTimeout(timer);
    timer = setTimeout(() => setDq(v.trim()), mode() === "contents" ? 300 : 150);
  };
  const wt = () => selectedWorktree();
  const info = () => findWorktree(wt());

  const [names] = createResource(
    () => ({ w: wt(), q: dq(), m: mode(), tick: fsTick() }),
    async ({ w, q, m }): Promise<string[]> => (w && q && m === "names" ? fsSearch(w, q) : []),
  );
  const [hits] = createResource(
    () => ({ w: wt(), q: dq(), m: mode() }),
    async ({ w, q, m }): Promise<GrepHit[]> => (w && q && m === "contents" ? fsGrep(w, q) : []),
  );
  // matches grouped per file, in the order they were found
  const grouped = () => {
    const g = new Map<string, GrepHit[]>();
    for (const h of hits() ?? []) g.set(h.path, [...(g.get(h.path) ?? []), h]);
    return [...g.entries()];
  };

  const rootName = () => info()?.wt.path.split(/[\\/]/).pop() ?? "";

  return (
    <aside class="rightbar" id="rpanel" role="tabpanel" aria-labelledby={`rtab-${rightTab()}`}>
      <RightTabs />
      <div class="rtools fp-head">
        <strong class="fp-title" title={info()?.project.name}>
          {info()?.project.name ?? P.files.title}
        </strong>
        <span class="grow" />
        <Button variant="icon" size="icon" title={P.files.collapse} aria-label={P.files.collapse} onClick={() => setExpanded(new Set([""]))}>
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 5h10M5 8.5h6M7 12h2" />
          </svg>
        </Button>
        <Button variant="icon" size="icon" title={P.files.refresh} aria-label={P.files.refresh} onClick={() => setGen(gen() + 1)}>
          <Icon name="refresh" />
        </Button>
      </div>
      <div class="fp-search">
        <Input
          class="fsearch"
          type="search"
          placeholder={P.files.searchPlaceholder}
          aria-label={P.files.searchLabel}
          value={q()}
          onInput={(e) => onSearch(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setQ("");
              setDq("");
            } else if (e.key === "Enter" && mode() === "names" && names()?.[0]) openPath(names()![0]);
          }}
        />
        <Tabs value={mode()} onChange={(v) => (setMode(v as "names" | "contents"), setDq(q().trim()))}>
          <TabsList variant="segmented" aria-label={P.files.searchIn}>
            <TabsTrigger variant="segmented" value="names">
              {P.files.names}
            </TabsTrigger>
            <TabsTrigger variant="segmented" value="contents">
              {P.files.contents}
            </TabsTrigger>
          </TabsList>
        </Tabs>
      </div>
      <div
        class="rlist"
        onContextMenu={(e) => {
          if (e.target === e.currentTarget && wt()) showMenu(e, fileMenu(wt()!, "", true).filter((i) => "separator" in i || NEW_ITEMS.includes(i.label)).slice(0, 2));
        }}
      >
        <Show when={dq() && mode() === "names"}>
          <For each={names() ?? []} fallback={<div class="empty">{names.loading ? P.files.searching : P.files.noFile}</div>}>
            {(f) => {
              const i = f.lastIndexOf("/");
              return (
                <div class="file" classList={{ active: openFile() === f }} {...press(() => openPath(f), "button", menuKey(() => fileMenu(wt()!, f, false)))} onContextMenu={(e) => showMenu(e, fileMenu(wt()!, f, false))} title={f}>
                  <FileGlyph />
                  <span class="fname2">
                    <span class="fn">{f.slice(i + 1)}</span>
                    <span class="fd">{i > 0 ? f.slice(0, i) : ""}</span>
                  </span>
                </div>
              );
            }}
          </For>
        </Show>
        <Show when={dq() && mode() === "contents"}>
          <For each={grouped()} fallback={<div class="empty">{hits.loading ? P.files.searching : P.files.noResults}</div>}>
            {([path, list]) => (
              <div class="ghit">
                <div class="file" {...press(() => openPath(path), "button", menuKey(() => fileMenu(wt()!, path, false)))} onContextMenu={(e) => showMenu(e, fileMenu(wt()!, path, false))} title={path}>
                  <FileGlyph />
                  <span class="fname2">
                    <span class="fn">{baseOf(path)}</span>
                    <span class="fd">{parentOf(path)}</span>
                  </span>
                  <span class="sc-count">{list.length}</span>
                </div>
                <For each={list.slice(0, 5)}>
                  {(h) => (
                    <div class="gline" {...press(() => openPath(path))} title={`${path}:${h.line}`}>
                      <span class="gnum">{h.line}</span>
                      <span class="gtext">{h.text}</span>
                    </div>
                  )}
                </For>
              </div>
            )}
          </For>
        </Show>
        <Show when={!dq()}>
          {/* keyed on the worktree so switching reloads the tree from disk */}
          <Show when={wt()} keyed>
            {(w) => (
              <>
                <div
                  class="file root"
                  {...press(() => toggle(""), "button", menuKey(() => fileMenu(w, "", true).filter((i) => "separator" in i || NEW_ITEMS.includes(i.label)).slice(0, 2)))}
                  aria-expanded={isOpen("")}
                  onContextMenu={(e) => showMenu(e, fileMenu(w, "", true).filter((i) => "separator" in i || NEW_ITEMS.includes(i.label)).slice(0, 2))}
                >
                  <span class="twisty">
                    <Chevron open={isOpen("")} />
                  </span>
                  <Folder open={isOpen("")} />
                  <span class="fname">{rootName()}</span>
                  <Delta t={changeIndex().byDir.get("")} folder />
                </div>
                <Show when={isOpen("")}>
                  <div class="kids kids-root">
                    <Dir wt={w} path="" depth={1} />
                  </div>
                </Show>
              </>
            )}
          </Show>
        </Show>
      </div>
    </aside>
  );
}
