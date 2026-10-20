import { For, Show } from "solid-js";
import { FileChange } from "./api";
import { changes, changesMode, openDiff, refreshChanges, setChangesMode, setOpenDiff } from "./store";

const LABEL: Record<string, string> = { M: "alterado", A: "novo", D: "apagado", U: "conflito" };

const split = (p: string) => {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i < 0 ? ["", p] : [p.slice(0, i + 1), p.slice(i + 1)];
};

export function ChangesPanel() {
  const files = () => changes()?.files ?? [];
  const total = () => files().reduce((a, f) => ({ add: a.add + f.added, del: a.del + f.removed }), { add: 0, del: 0 });

  return (
    <aside class="rightbar">
      <div class="rtabs">
        <span class="rtab disabled">Ficheiros</span>
        <span class="rtab active">
          Alterações<Show when={files().length}> {files().length}</Show>
        </span>
        <span class="rtab disabled">Revisão</span>
      </div>
      <div class="rtools">
        <select
          value={changesMode()}
          onChange={(e) => {
            setChangesMode(e.currentTarget.value as "head" | "main");
            void refreshChanges();
          }}
        >
          <option value="head">Por consolidar (vs HEAD)</option>
          <option value="main">Todo o branch (vs principal)</option>
        </select>
        <button class="icon" title="Atualizar" onClick={() => void refreshChanges()}>
          ↻
        </button>
      </div>
      <div class="rsummary">
        {files().length} ficheiros · <span class="add">+{total().add}</span> <span class="del">−{total().del}</span>
      </div>
      <div class="rlist">
        <For each={files()} fallback={<div class="empty">Sem alterações</div>}>
          {(f: FileChange) => {
            const [dir, name] = split(f.path);
            return (
              <div class="file" classList={{ active: openDiff() === f.path }} onClick={() => setOpenDiff(f.path)} title={`${f.path} (${LABEL[f.status] ?? f.status})`}>
                <span class={`st st-${f.status}`}>{f.status}</span>
                <span class="fname">
                  <span class="fdir">{dir}</span>
                  {name}
                </span>
                <Show when={!f.binary} fallback={<span class="fbin">bin</span>}>
                  <span class="nums">
                    <Show when={f.added > 0}>
                      <span class="add">+{f.added}</span>
                    </Show>
                    <Show when={f.removed > 0}>
                      <span class="del">−{f.removed}</span>
                    </Show>
                  </span>
                </Show>
              </div>
            );
          }}
        </For>
      </div>
    </aside>
  );
}
