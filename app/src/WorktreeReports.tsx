import { For, Show, createResource } from "solid-js";
import { Button } from "./ui";
import { Icon } from "./Icon";
import { NoteView, openNoteFile } from "./NoteView";
import { notesWorktree } from "./api";
import { ago } from "./time";
import { U } from "./strings.ui";

/** Reports left by the agents that worked in a worktree with no session now: shown in its empty state. */
export function WorktreeReports(props: { project: string; worktree: string }) {
  const [notes] = createResource(
    () => ({ project: props.project, worktree: props.worktree }),
    (k) => notesWorktree(k.project, k.worktree).catch(() => []),
  );
  return (
    <>
      <Show when={notes()?.length}>
        <div class="wt-reports" role="list" aria-label={U.reports.list}>
          <For each={notes()}>
            {(n) => (
              <Button type="button" variant="row" size="bare" class="flex-col items-stretch gap-[3px] rounded-xl border-border bg-popover px-2.5 py-2 text-sm text-foreground hover:border-primary" role="listitem" onClick={() => openNoteFile(props.project, n.name)} title={U.reports.open}>
                <span class="wt-report-head">
                  <Icon name="file" size={12} inline />
                  <span class="wt-report-name">{n.name}</span>
                  <span class="wt-report-status" classList={{ ok: n.status === "ok" }}>{U.reports.status[n.status]}</span>
                  <span class="wt-report-time">{ago(Date.now() / 1000 - n.modified)}</span>
                </span>
                <Show when={n.excerpt}>
                  <span class="wt-report-excerpt">{n.excerpt}</span>
                </Show>
              </Button>
            )}
          </For>
        </div>
      </Show>
      <NoteView files />
    </>
  );
}
