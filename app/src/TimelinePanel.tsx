import { For, Show, createResource, createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { gitTimeline } from "./api";
import { errorText } from "./appError";
import { Icon } from "./Icon";
import { Loading } from "./Loading";
import { RightTabs } from "./RightTabs";
import { findWorktree, rightTab, selectedWorktree, setError, showNotice } from "./store";
import { S } from "./strings";
import { P } from "./strings.panels";
import { ciLine, eventTitle, stamp, toMarkdown, tokenLine } from "./timelineLogic";
import { Badge, Button, cn } from "./ui";

const T = P.timeline;
/** What happened in the selected worktree: commits, merges, agent reports, with the tokens and CI as totals; copyable as markdown. */
export function TimelinePanel() {
  const [withCi, setWithCi] = createSignal(false);
  const current = () => findWorktree(selectedWorktree());
  const [data, { refetch }] = createResource(
    () => {
      const c = current();
      return c ? { wt: c.wt.path, project: c.project.id, ci: withCi() } : null;
    },
    (k) => gitTimeline(k.wt, k.project, k.ci),
  );

  const copy = async () => {
    const t = data();
    const c = current();
    if (!t || !c) return;
    try {
      await invoke("clipboard_write", {
        text: toMarkdown(t, c.wt.path.split(/[/\\]/).filter(Boolean).pop() ?? c.wt.path),
      });
      showNotice(T.copied);
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <aside class="rightbar" id="rpanel" role="tabpanel" aria-labelledby={`rtab-${rightTab()}`}>
      <RightTabs />
      <div class="rtools">
        <Button
          variant="icon"
          size="sm"
          class={cn("min-w-0 shrink truncate", withCi() && "text-primary")}
          aria-pressed={withCi()}
          title={T.withCiTip}
          onClick={() => setWithCi(!withCi())}
        >
          {T.withCi}
        </Button>
        <span class="grow" />
        <Button
          variant="icon"
          size="icon"
          disabled={!data()}
          title={T.copy}
          aria-label={T.copy}
          onClick={() => void copy()}
        >
          <Icon name="copy" />
        </Button>
        <Button variant="icon" size="icon" title={T.refresh} aria-label={T.refresh} onClick={() => void refetch()}>
          <Icon name="refresh" />
        </Button>
      </div>
      <div class="rlist" aria-label={T.title}>
        <Show when={current()} fallback={<div class="placeholder">{T.nothing}</div>}>
          <Show when={!data.loading} fallback={<Loading onRetry={() => void refetch()} />}>
            <Show
              when={!data.error}
              fallback={
                <div class="placeholder" role="alert">
                  <span>{errorText(data.error)}</span>
                  <div>
                    <Button size="sm" onClick={() => void refetch()}>
                      {S.retry}
                    </Button>
                  </div>
                </div>
              }
            >
              <Show when={data()}>
                {(t) => (
                  <>
                    <div class="pr">
                      <div class="pr-meta">
                        <Badge class="mono">{t().branch}</Badge>
                        <Show when={t().tokens}>{(k) => <span title={T.tokensLabel}>{tokenLine(k())}</span>}</Show>
                      </div>
                      <Show when={t().ci}>{(ci) => <div class="pr-meta">{`${T.ciLabel}: ${ciLine(ci())}`}</div>}</Show>
                    </div>
                    <For each={t().events} fallback={<div class="placeholder">{T.empty}</div>}>
                      {(e) => (
                        <div class="pr">
                          <div class="pr-title">
                            <Badge>{T.kinds[e.kind] ?? e.kind}</Badge>
                            <span class="pr-text" title={eventTitle(e)}>
                              {eventTitle(e)}
                            </span>
                          </div>
                          <div class="pr-meta">
                            <span>{stamp(e.at)}</span>
                            <Show when={e.detail}>
                              <span class="pr-text" title={e.detail}>
                                {e.detail}
                              </span>
                            </Show>
                          </div>
                        </div>
                      )}
                    </For>
                  </>
                )}
              </Show>
            </Show>
          </Show>
        </Show>
      </div>
    </aside>
  );
}
