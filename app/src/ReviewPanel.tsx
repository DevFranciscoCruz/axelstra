import { RightTabs } from "./RightTabs";
import { For, Show, createResource } from "solid-js";
import { Commit, gitLog } from "./api";
import { changes, findWorktree, openCommit, selectedWorktree, setOpenCommit, setOpenDiff, setOpenFile, setRightTab, setShowKanban } from "./store";

/** Commits this branch has over the project's main branch (or the latest ones on the main worktree). */
export const reviewBase = (): string | undefined => {
  const c = findWorktree(selectedWorktree());
  if (!c || c.wt.is_main) return undefined;
  return c.project.default_branch ?? c.project.worktrees.find((w) => w.is_main)?.branch ?? undefined;
};

export function ReviewPanel() {
  const key = () => ({ wt: selectedWorktree(), base: reviewBase() });
  const [log, { refetch }] = createResource(key, (k) => (k.wt ? gitLog(k.wt, k.base) : []));
  const nChanges = () => changes()?.files.length ?? 0;
  const open = (c: Commit) => {
    setOpenDiff(null);
    setOpenFile(null);
    setShowKanban(false);
    setOpenCommit(c);
  };
  const when = (iso: string) => iso.slice(0, 16).replace("T", " ");

  return (
    <aside class="rightbar">
      <RightTabs />
      <div class="rtools">
        <span class="rsummary">{reviewBase() ? `Commits à frente de ${reviewBase()}` : "Últimos commits"}</span>
        <span class="grow" />
        <button class="icon" title="Atualizar" onClick={() => void refetch()}>
          ↻
        </button>
      </div>
      <div class="rlist">
        <Show when={log.error}>
          <div class="empty">{String(log.error)}</div>
        </Show>
        <For each={log() ?? []} fallback={<Show when={!log.loading}><div class="empty">Sem commits para rever</div></Show>}>
          {(c) => (
            <div class="commit" classList={{ active: openCommit()?.sha === c.sha }} onClick={() => open(c)} title={c.subject}>
              <div class="csubject">{c.subject}</div>
              <div class="cmeta">
                <span class="mono">{c.short}</span> · {c.author} · {when(c.date)}
                <span class="nums">
                  {c.files} <span class="add">+{c.added}</span> <span class="del">−{c.removed}</span>
                </span>
              </div>
            </div>
          )}
        </For>
      </div>
    </aside>
  );
}
