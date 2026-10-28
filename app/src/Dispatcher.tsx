import { For, Show, createMemo, onCleanup } from "solid-js";
import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { StateDot } from "./StateDot";
import { dispatchTopics, norm, projects, selectWorktree, sessionState, sessionsOf, setShowDispatcher, watchDispatch } from "./store";
import { Button } from "./ui";
import { S } from "./strings";
import { U } from "./strings.ui";
import { P } from "./strings.panels";

/** Live view of what the main session dispatched: one card per topic (worktree) with its latest requests. */
export function Dispatcher() {
  // the store polls once for everyone who shows the topics (and not while the window is hidden)
  onCleanup(watchDispatch());
  const meta = dispatchTopics;

  const topics = createMemo(() => {
    const out: { project: string; path: string; branch: string | null; title: string; requests: string[]; archived: boolean; updated: number }[] = [];
    for (const p of projects()) {
      for (const w of p.worktrees) {
        const m = meta()[norm(w.path)];
        if (!m || w.is_main) continue;
        out.push({ project: p.name, path: w.path, branch: w.branch, title: m.title ?? w.branch ?? w.path, requests: m.requests ?? [], archived: !!m.archived, updated: m.updated ?? 0 });
      }
    }
    return out.sort((a, b) => Number(a.archived) - Number(b.archived) || b.updated - a.updated);
  });

  const stateOf = (path: string) => {
    const agents = sessionsOf(path).filter((s) => s.meta?.kind === "agent");
    if (agents.some((s) => sessionState(s) === "waiting")) return { dot: "waiting" as const, text: U.dispatcher.waiting };
    if (agents.some((s) => sessionState(s) === "working")) return { dot: "working" as const, text: U.dispatcher.working };
    if (agents.some((s) => s.running)) return { dot: "idle" as const, text: U.dispatcher.idle };
    return { dot: "none" as const, text: U.dispatcher.noAgent };
  };

  return (
    <Modal label={U.dispatcher.title} class="modal settings" onClose={() => setShowDispatcher(false)}>
      <div class="auto-head">
        <strong>{U.dispatcher.title}</strong>
        <span class="grow" />
        <Button onClick={() => setShowDispatcher(false)}>{S.close}</Button>
      </div>
      <p class="auto-note">{U.dispatcher.note}</p>
      <div class="disp-list">
        <For each={topics()} fallback={<div class="auto-note">{U.dispatcher.empty}</div>}>
          {(t) => {
            const st = () => stateOf(t.path);
            return (
              <div class="disp-card" classList={{ archived: t.archived }}>
                <div class="disp-head">
                  <StateDot state={st().dot} />
                  <strong class="disp-title">{t.title}</strong>
                  <span class="auto-note">{t.archived ? U.dispatcher.archived : st().text}</span>
                  <span class="grow" />
                  <Button
                    size="icon"
                    variant="ghost"
                    title={U.dispatcher.openWorktree}
                    aria-label={U.dispatcher.openWorktree}
                    onClick={() => {
                      selectWorktree(t.path);
                      setShowDispatcher(false);
                    }}
                  >
                    <Icon name="external" />
                  </Button>
                </div>
                <div class="auto-note">{t.project} · {t.branch ?? S.detached}</div>
                <Show when={t.requests.length > 0}>
                  <ol class="disp-reqs">
                    <For each={t.requests.slice(-3).reverse()}>{(r) => <li>{r}</li>}</For>
                  </ol>
                  <Show when={t.requests.length > 3}>
                    <div class="auto-note">{P.flows.moreRequests(t.requests.length - 3)}</div>
                  </Show>
                </Show>
              </div>
            );
          }}
        </For>
      </div>
    </Modal>
  );
}
