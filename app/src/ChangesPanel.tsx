import { ask } from "@tauri-apps/plugin-dialog";
import { For, Show, createSignal } from "solid-js";
import { FileChange, gitCommit, gitPr, gitPush } from "./api";
import { baseRef, changes, changesMode, findWorktree, selectedWorktree, openDiff, refreshChanges, setChangesMode, setOpenDiff, setOpenFile, setRightTab } from "./store";

const LABEL: Record<string, string> = { M: "alterado", A: "novo", D: "apagado", U: "conflito" };

const split = (p: string) => {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i < 0 ? ["", p] : [p.slice(0, i + 1), p.slice(i + 1)];
};

export function ChangesPanel() {
  const files = () => changes()?.files ?? [];
  const total = () => files().reduce((a, f) => ({ add: a.add + f.added, del: a.del + f.removed }), { add: 0, del: 0 });

  const [message, setMessage] = createSignal("");
  const [prOpen, setPrOpen] = createSignal(false);
  const [prTitle, setPrTitle] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [note, setNote] = createSignal<{ ok: boolean; text: string } | null>(null);

  const act = async (f: () => Promise<string>) => {
    setBusy(true);
    setNote(null);
    try {
      setNote({ ok: true, text: await f() });
    } catch (e) {
      setNote({ ok: false, text: String(e) });
    } finally {
      setBusy(false);
      void refreshChanges();
    }
  };
  const branch = () => findWorktree(selectedWorktree())?.wt.branch ?? "";
  const doCommit = () =>
    act(async () => {
      const r = await gitCommit(selectedWorktree()!, message());
      setMessage("");
      return `Commit ${r.sha}`;
    });
  const doPush = async () => {
    // pushing publishes the branch to the remote: always confirm
    if (!(await ask(`Enviar o branch "${branch()}" para o remote origin?`, { title: "Push", kind: "warning" }))) return;
    await act(async () => {
      await gitPush(selectedWorktree()!);
      return `Push de ${branch()} feito`;
    });
  };
  const doPr = async () => {
    if (!(await ask(`Criar um pull request de "${branch()}" no GitHub?`, { title: "Pull request", kind: "warning" }))) return;
    await act(async () => {
      const url = await gitPr(selectedWorktree()!, prTitle(), "", changesMode() === "main" ? baseRef() : undefined);
      setPrOpen(false);
      setPrTitle("");
      return url;
    });
  };

  return (
    <aside class="rightbar">
      <div class="rtabs">
        <span class="rtab" onClick={() => setRightTab("files")}>Ficheiros</span>
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
              <div class="file" classList={{ active: openDiff() === f.path }} onClick={() => {
                  setOpenFile(null);
                  setOpenDiff(f.path);
                }} title={`${f.path} (${LABEL[f.status] ?? f.status})`}>
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
      <div class="publish">
        <textarea
          placeholder="Mensagem do commit"
          rows={2}
          value={message()}
          onInput={(e) => setMessage(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && e.ctrlKey && files().length > 0 && message().trim()) void doCommit();
          }}
        />
        <div class="prow">
          <button class="primary" disabled={busy() || files().length === 0 || !message().trim()} onClick={() => void doCommit()}>
            Commit ({files().length})
          </button>
          <button disabled={busy()} title="git push -u origin" onClick={() => void doPush()}>
            Push
          </button>
          <button disabled={busy()} onClick={() => setPrOpen(!prOpen())}>
            PR…
          </button>
        </div>
        <Show when={prOpen()}>
          <div class="prow">
            <input
              placeholder="Título do pull request"
              value={prTitle()}
              onInput={(e) => setPrTitle(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && prTitle().trim() && void doPr()}
            />
            <button disabled={busy() || !prTitle().trim()} onClick={() => void doPr()}>
              Criar
            </button>
          </div>
        </Show>
        <Show when={note()}>
          {(n) => (
            <div class="pnote" classList={{ bad: !n().ok }} title={n().text}>
              {n().text}
            </div>
          )}
        </Show>
      </div>
    </aside>
  );
}
