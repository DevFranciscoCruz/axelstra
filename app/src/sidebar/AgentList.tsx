import { For, Show, createSignal } from "solid-js";
import { Icon, hasIcon } from "../Icon";
import { Button, Input } from "../ui";
import { StateDot } from "../StateDot";
import { openNote } from "../NoteView";
import { ago } from "../time";
import { DOT_LABEL, dotOfSession, dotTip } from "../agentState";
import { agentRowInfo, agentRowLabel } from "../agentRow";
import { isLead, lacksLead, leadFirst } from "../leadRole";
import { P } from "../strings.panels";
import { MenuItem, isMenuKey, showMenu, showMenuAt } from "../ContextMenu";
import { createSidebarActions } from "./actions";
import { agentMenu } from "./menus";
import {
  activeSession,
  closeSessionTab,
  norm,
  openSession,
  renameTab,
  selectedWorktree,
  sessionsOf,
  tabLabel,
} from "../store";
import type { SessionInfo } from "../api";

const T = P.sidebar;

const idleLabel = (ms?: number) => (ms === undefined ? "" : ago(ms / 1000));

/** The agents of one worktree under its row: folds away, opens a tab, renames in place with F2, closes with the middle button. */
export function AgentList(props: {
  path: string;
  depth: number;
  hidden: boolean;
  onToggle: () => void;
  /** the project's main checkout has no lead */
  isMain?: boolean;
  onStartLead?: () => void;
}) {
  const [renaming, setRenaming] = createSignal<number | null>(null);
  const actions = createSidebarActions();
  const menuOf = (a: SessionInfo): MenuItem[] => {
    const wt = a.meta?.worktree ?? props.path;
    return agentMenu(
      { run: actions.run, copy: actions.copy, open: () => openSession(wt, a.id), end: () => void closeSessionTab(a) },
      a.id,
      wt,
    );
  };
  // agents of this worktree; workers that an agent of another worktree started show in their own worktree
  // the lead first, its tasks after it
  const agents = () => leadFirst(sessionsOf(props.path).filter((x) => x.meta?.kind === "agent"));
  const hasLead = () => agents().some((a) => a.running && isLead(a));
  const missingLead = () => !!props.onStartLead && lacksLead(agents(), !!props.isMain);
  const finishRename = (id: number, name: string | null) => {
    if (name !== null) renameTab(id, name);
    setRenaming(null);
    queueMicrotask(() => document.querySelector<HTMLElement>(`[data-agent-row="${id}"]`)?.focus());
  };
  return (
    <Show when={agents().length > 0 || missingLead()}>
      <div class="wt-agents" style={props.depth > 0 ? { "margin-left": `${14 + props.depth * 14}px` } : undefined}>
        <Show when={missingLead()}>
          <Button
            type="button"
            variant="link"
            size="bare"
            class="w-full justify-start gap-1.5 px-1.5 py-0.5 text-xs"
            title={T.startLeadTip}
            onClick={() => props.onStartLead?.()}
          >
            <Icon name="plus" size={11} />
            {T.startLead}
          </Button>
        </Show>
        <Show when={agents().length > 0}>
          <Button
            variant="link"
            size="bare"
            class="w-full justify-between px-1.5 pb-1 pt-0.5 text-xs"
            onClick={props.onToggle}
            aria-expanded={!props.hidden}
          >
            <span>{T.agentsCount(agents().length)}</span>
            <Icon name={props.hidden ? "chevRight" : "chevDown"} size={11} />
          </Button>
        </Show>
        <Show when={agents().length > 0 && !props.hidden}>
          <For each={agents()}>
            {(a) => {
              const info = () => agentRowInfo(a, props.path, tabLabel(a));
              const state = () => dotTip(a) ?? DOT_LABEL[dotOfSession(a)];
              const active = () => activeSession() === a.id && norm(selectedWorktree()) === norm(props.path);
              return (
                <div
                  class="wt-agent"
                  classList={{ active: active(), lead: isLead(a), task: !isLead(a) && hasLead() }}
                  onContextMenu={(e) => {
                    if (renaming() !== a.id) showMenu(e, menuOf(a));
                  }}
                >
                  <Show
                    when={renaming() !== a.id}
                    fallback={
                      <Input
                        class="mx-1 my-0.5 h-auto min-w-0 flex-1 px-1.5 py-[3px] text-sm"
                        aria-label={T.agentName}
                        value={info().name}
                        ref={(el) =>
                          queueMicrotask(() => {
                            el.focus();
                            el.select();
                          })
                        }
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
                        if (isMenuKey(e)) {
                          e.preventDefault();
                          e.stopPropagation();
                          const r = e.currentTarget.getBoundingClientRect();
                          showMenuAt(r.left + 24, r.bottom, menuOf(a));
                          return;
                        }
                        if (e.key === "F2") {
                          e.preventDefault();
                          setRenaming(a.id);
                        }
                      }}
                    >
                      <StateDot state={dotOfSession(a)} tip={dotTip(a)} />
                      <span class="wt-agent-name">
                        {a.meta?.worker ? <Icon name="subArrow" size={11} inline /> : null}
                        {info().name}
                        {isLead(a) ? (
                          <span class="wt-lead-tag" role="img" title={T.leadTip} aria-label={T.leadTip}>
                            <Icon name="star" size={11} inline />
                          </span>
                        ) : null}
                      </span>
                      <Show when={info().kind}>
                        <span
                          class="wt-agent-kind inline-flex items-center text-muted-foreground"
                          title={info().kind}
                          role="img"
                          aria-label={T.agentKind(info().kind)}
                        >
                          {hasIcon(info().kind) ? <Icon name={info().kind} size={13} /> : info().kind}
                        </span>
                      </Show>
                      <Show when={info().worktree}>
                        <span
                          class="wt-agent-wt inline-flex items-center text-muted-foreground"
                          title={info().worktree}
                          role="img"
                          aria-label={T.worktreeOf(info().worktree)}
                        >
                          <Icon name="branch" size={12} />
                        </span>
                      </Show>
                    </Button>
                  </Show>
                  <Show when={info().hasReport && renaming() !== a.id}>
                    <Button
                      type="button"
                      variant="chip"
                      size="chip"
                      class="mr-1 flex-none px-[6px]"
                      aria-label={T.reportOf(info().name)}
                      title={T.openReport}
                      onClick={() => openNote(a.id, info().name)}
                    >
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
