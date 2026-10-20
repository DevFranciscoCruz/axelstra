import { For, Show, createSignal } from "solid-js";
import { FsEntry, fsList } from "./api";
import { changes, openFile, selectedWorktree, setOpenDiff, setOpenFile, setRightTab } from "./store";

const join = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

function Dir(props: { wt: string; path: string; depth: number }) {
  const [entries, setEntries] = createSignal<FsEntry[] | null>(null);
  const [err, setErr] = createSignal("");
  void fsList(props.wt, props.path)
    .then(setEntries)
    .catch((e) => setErr(String(e)));
  return (
    <>
      <Show when={err()}>
        <div class="empty">{err()}</div>
      </Show>
      <For each={entries() ?? []}>{(e) => <Node wt={props.wt} dir={props.path} e={e} depth={props.depth} />}</For>
    </>
  );
}

function Node(props: { wt: string; dir: string; e: FsEntry; depth: number }) {
  const [open, setOpen] = createSignal(false);
  const path = () => join(props.dir, props.e.name);
  const pad = () => ({ "padding-left": `${8 + props.depth * 12}px` });
  return (
    <>
      <div
        class="file"
        classList={{ active: !props.e.dir && openFile() === path() }}
        style={pad()}
        onClick={() => {
          if (props.e.dir) return setOpen(!open());
          setOpenDiff(null);
          setOpenFile(path());
        }}
      >
        <span class="twisty">{props.e.dir ? (open() ? "▾" : "▸") : ""}</span>
        <span class="fname">{props.e.name}</span>
      </div>
      <Show when={props.e.dir && open()}>
        <Dir wt={props.wt} path={path()} depth={props.depth + 1} />
      </Show>
    </>
  );
}

export function FilesPanel() {
  const [gen, setGen] = createSignal(0);
  const nChanges = () => changes()?.files.length ?? 0;
  return (
    <aside class="rightbar">
      <div class="rtabs">
        <span class="rtab active">Ficheiros</span>
        <span class="rtab" onClick={() => setRightTab("changes")}>
          Alterações<Show when={nChanges()}> {nChanges()}</Show>
        </span>
        <span class="rtab disabled">Revisão</span>
      </div>
      <div class="rtools">
        <span class="grow" />
        <button class="icon" title="Atualizar" onClick={() => setGen(gen() + 1)}>
          ↻
        </button>
      </div>
      <div class="rlist">
        {/* keyed on worktree + refresh counter so the tree reloads from disk */}
        <Show when={`${selectedWorktree()}|${gen()}`} keyed>
          {(_k) => <Show when={selectedWorktree()}>{(wt) => <Dir wt={wt()} path="" depth={0} />}</Show>}
        </Show>
      </div>
    </aside>
  );
}
