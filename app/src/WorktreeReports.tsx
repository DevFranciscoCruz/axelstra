import { For, Show, createResource } from "solid-js";
import { Icon } from "./Icon";
import { NoteView, openNoteFile } from "./NoteView";
import { notesWorktree } from "./api";
import { ago } from "./time";

const STATUS = { ok: "concluído", nota: "com notas", aberto: "sem relatório" } as const;

/** Reports left by the agents that worked in a worktree with no session now: shown in its empty state. */
export function WorktreeReports(props: { project: string; worktree: string }) {
  const [notes] = createResource(
    () => ({ project: props.project, worktree: props.worktree }),
    (k) => notesWorktree(k.project, k.worktree).catch(() => []),
  );
  return (
    <>
      <Show when={notes()?.length}>
        <div class="wt-reports" role="list" aria-label="Relatórios dos agentes deste worktree">
          <For each={notes()}>
            {(n) => (
              <button type="button" class="wt-report" role="listitem" onClick={() => openNoteFile(props.project, n.name)} title="Abrir o relatório">
                <span class="wt-report-head">
                  <Icon name="file" size={12} inline />
                  <span class="wt-report-name">{n.name}</span>
                  <span class="wt-report-status" classList={{ ok: n.status === "ok" }}>{STATUS[n.status]}</span>
                  <span class="wt-report-time">{ago(Date.now() / 1000 - n.modified)}</span>
                </span>
                <Show when={n.excerpt}>
                  <span class="wt-report-excerpt">{n.excerpt}</span>
                </Show>
              </button>
            )}
          </For>
        </div>
      </Show>
      <NoteView files />
    </>
  );
}
