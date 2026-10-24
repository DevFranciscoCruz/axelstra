import { For, Show, createSignal } from "solid-js";
import { Icon, hasIcon } from "../Icon";
import { Button, Input } from "../ui";
import { StateDot } from "../StateDot";
import { openNote } from "../NoteView";
import { ago } from "../time";
import { DOT_LABEL, dotOfSession } from "../agentState";
import { agentRowInfo, agentRowLabel } from "../agentRow";
import { P } from "../strings.panels";
import { activeSession, closeSessionTab, norm, openSession, renameTab, selectedWorktree, sessionsOf, tabLabel } from "../store";

const T = P.sidebar;

const idleLabel = (ms?: number) => (ms === undefined ? "" : ago(ms / 1000));

/** The agents of one worktree under its row: folds away, opens a tab, renames in place with F2, closes with the middle button. */
export function AgentList(props: { path: string; depth: number; hidden: boolean; onToggle: () => void }) {
  const [renaming, setRenaming] = createSignal<number | null>(null);
  // agents of this worktree; workers that an agent of another worktree started show in their own worktree
  const agents = () => sessionsOf(props.path).filter((x) => x.meta?.kind === "agent");
  const finishRename = (id: number, name: string | null) => {
    if (name !== null) renameTab(id, name);
    setRenaming(null);
    queueMicrotask(() => document.querySelector<HTMLElement>(`[data-agent-row="${id}"]`)?.focus());
  };
  return (
    <Show when={agents().length > 0}>
      <div class="wt-agents" style={props.depth > 0 ? { "margin-left": `${14 + props.depth * 14}px` } : undefined}>
        <Button variant="link" size="bare" class="w-full justify-between px-1.5 pb-1 pt-0.5 text-xs" onClick={props.onToggle} aria-expanded={!props.hidden}>
          <span>{T.agentsCount(agents().length)}</span>
          <Icon name={props.hidden ? "chevRight" : "chevDown"} size={11} />
        </Button>
        <Show when={!props.hidden}>
          <For each={agents()}>
            {(a) => {
              const info = () => agentRowInfo(a, props.path, tabLabel(a));
              const state = () => DOT_LABEL[dotOfSession(a)];
              const active = () => activeSession() === a.id && norm(selectedWorktree()) === norm(props.path);
              return (
                <div class="wt-agent" classList={{ active: active() }}>
                  <Show
                    when={renaming() !== a.id}
                    fallback={
                      <Input
                        class="mx-1 my-0.5 h-auto min-w-0 flex-1 px-1.5 py-[3px] text-sm"
                        aria-label={T.agentName}
                        value={info().name}
                        ref={(el) => queueMicrotask(() => { el.focus(); el.select(); })}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") finishRename(a.id, e.currentTarget.value);
                          else if (e.key === "Escape") finishRename(a.id, null);
                        }}
                        onBlur={(e) => renaming() === a.id && finishRename(a.id, e.currentTarget.value)}
                      />
                    }
                  >
                    <Button
                      type="button"
                      variant="row"
                      size="bare"
                      class="min-w-0 flex-1 gap-2 px-1.5 py-1"
                      data-agent-row={a.id}
                      aria-label={agentRowLabel(info(), state())}
                      aria-current={active() ? "true" : undefined}
                      title={T.agentRenameTip(info().name)}
                      onClick={() => openSession(a.meta?.worktree ?? props.path, a.id)}
                      onMouseDown={(e) => e.button === 1 && e.preventDefault()}
                      onAuxClick={(e) => {
                        if (e.button !== 1) return;
                        e.preventDefault();
                        void closeSessionTab(a);
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "F2") {
                          e.preventDefault();
                          setRenaming(a.id);
                        }
                      }}
                    >
                      <StateDot state={dotOfSession(a)} />
                      <span class="wt-agent-name">{a.meta?.worker ? <Icon name="subArrow" size={11} inline /> : null}{info().name}</span>
                      <Show when={info().kind}>
                        <span class="wt-agent-kind inline-flex items-center text-muted-foreground" title={info().kind} role="img" aria-label={T.agentKind(info().kind)}>{hasIcon(info().kind) ? <Icon name={info().kind} size={13} /> : info().kind}</span>
                      </Show>
                      <Show when={info().worktree}>
                        <span class="wt-agent-wt inline-flex items-center text-muted-foreground" title={info().worktree} role="img" aria-label={T.worktreeOf(info().worktree)}><Icon name="branch" size={12} /></span>
                      </Show>
                    </Button>
                  </Show>
                  <Show when={info().hasReport && renaming() !== a.id}>
                    <Button type="button" variant="chip" size="chip" class="mr-1 flex-none px-[6px]" aria-label={T.reportOf(info().name)} title={T.openReport} onClick={() => openNote(a.id, info().name)}>
                      <Icon name="file" size={12} />
                    </Button>
                  </Show>
                  <span class="wt-agent-time">{a.running ? idleLabel(a.idle_ms) : T.ended}</span>
                </div>
              );
            }}
          </For>
        </Show>
      </div>
    </Show>
  );
}
