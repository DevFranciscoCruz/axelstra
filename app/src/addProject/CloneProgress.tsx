import { Show } from "solid-js";
import { S } from "../strings";
import { AP } from "../strings.sources";
import { Button } from "../ui";
import { cancelClone, cloneActive, cloneJob, dismissClone } from "./cloneJob";

/** Progress of the clone in flight and how it ended, for the tabs that clone (announced to screen readers; Cancel kills git and removes the partial folder). */
export function CloneProgress() {
  return (
    <Show when={cloneJob()}>
      {(j) => (
        <div class="flex flex-col gap-1.5 rounded-md border border-border p-2">
          <div role="status" aria-live="polite" class="flex flex-col gap-1 text-base">
            <Show when={cloneActive(j())}>
              <span>{j().phase || AP.starting}</span>
              <Show
                when={j().percent != null}
                fallback={<progress class="h-1.5 w-full" aria-label={AP.progress} max="100" />}
              >
                <progress class="h-1.5 w-full" aria-label={AP.progress} max="100" value={j().percent} />
              </Show>
              <span class="auto-note">{AP.background}</span>
            </Show>
            <Show when={j().state === "done"}>
              <span>{AP.cloned(j().name)}</span>
            </Show>
            <Show when={j().state === "cancelled"}>
              <span>{AP.cloneCancelled}</span>
            </Show>
          </div>
          <Show when={j().state === "failed"}>
            <div role="alert" class="text-[var(--err)]">
              {AP.cloneFailed(j().error ?? "")}
            </div>
          </Show>
          <div class="m-actions">
            <Show
              when={cloneActive(j())}
              fallback={
                <Button type="button" onClick={dismissClone}>
                  {S.close}
                </Button>
              }
            >
              <Button type="button" onClick={() => void cancelClone()}>
                {AP.cancelClone}
              </Button>
            </Show>
          </div>
        </div>
      )}
    </Show>
  );
}
