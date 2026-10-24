import { Show } from "solid-js";
import { Icon } from "../Icon";
import { Badge, Button } from "../ui";
import { StateDot } from "../StateDot";
import { MenuItem, isMenuKey, showMenu, showMenuAt } from "../ContextMenu";
import { dotOfWorktree } from "../agentState";
import { worktreeSignals } from "../leadRole";
import { P } from "../strings.panels";
import {
  attention,
  flowState,
  isPinned,
  norm,
  prByBranch,
  prKey,
  selectedWorktree,
  sessionState,
  sessionsOf,
  wtStatus,
} from "../store";
import { AgentList } from "./AgentList";

const T = P.sidebar;

type Wt = { path: string; branch: string | null; is_main: boolean };
type Proj = { id: string; default_branch?: string | null };
type Topic = { title?: string; archived?: boolean } | undefined;

/** One worktree under its project: state dot, badges, the "more" and remove buttons, and its agents. */
export function WorktreeRow(props: {
  project: Proj;
  wt: Wt;
  depth: number;
  kids: number;
  folded: boolean;
  /** running agents of the whole group, shown while its children are folded away */
  groupLive: number;
  topic: Topic;
  /** titles of the "same files" badge, null when nothing overlaps */
  overlap: string | null;
  /** distinct files other worktrees also edit */
  overlapCount: number;
  agentsHidden: boolean;
  menu: (label: string) => MenuItem[];
  onOpen: () => void;
  onToggleFold: () => void;
  onToggleAgents: () => void;
  onStartLead: () => void;
  onDrop: (label: string) => void;
}) {
  const w = () => props.wt;
  const label = () => w().branch ?? P.sidebar.detached;
  const mainBranch = () => props.project.default_branch ?? "main";
  // the lead speaks for the worktree; a task only adds "waiting"
  const signals = () => worktreeSignals(sessionsOf(w().path), sessionState);
  const live = () => signals().live;
  const working = () => signals().working;
  const waiting = () => signals().waiting;
  const needs = () => attention().has(norm(w().path));
  const queued = () => live() === 0 && flowState()[norm(w().path)] === "queued";
  const st = () => wtStatus()[norm(w().path)];
  const pr = () => prByBranch()[prKey(props.project.id, w().branch)];
  const isActive = () => norm(selectedWorktree()) === norm(w().path);
  return (
    <>
      <div
        class="item wt"
        classList={{ active: isActive(), child: props.depth > 0 }}
        style={props.depth > 0 ? { "margin-left": `${props.depth * 14}px` } : undefined}
        onKeyDown={(e) => {
          // the menu key and Shift+F10 open the row menu from the keyboard
          if (isMenuKey(e)) {
            e.preventDefault();
            const r = e.currentTarget.getBoundingClientRect();
            showMenuAt(r.left + 24, r.bottom, props.menu(label()));
            return;
          }
          // left folds a group, right opens it
          if (
            props.kids > 0 &&
            (e.target as HTMLElement).classList.contains("wt-open") &&
            ((e.key === "ArrowLeft" && !props.folded) || (e.key === "ArrowRight" && props.folded))
          ) {
            e.preventDefault();
            props.onToggleFold();
          }
        }}
        onContextMenu={(e) => showMenu(e, props.menu(label()))}
        title={w().path}
      >
        <Show when={props.kids > 0}>
          <Button
            type="button"
            variant="link"
            size="bare"
            class="h-[18px] w-4 flex-none rounded-sm"
            aria-expanded={!props.folded}
            aria-label={props.folded ? T.openGroup(label()) : T.closeGroup(label())}
            title={props.folded ? T.showGroup : T.hideGroup}
            onClick={(e) => {
              e.stopPropagation();
              props.onToggleFold();
            }}
          >
            <Icon name={props.folded ? "chevRight" : "chevDown"} size={11} />
          </Button>
        </Show>
        <Button
          type="button"
          variant="row"
          size="bare"
          class="min-w-0 flex-1 gap-2"
          aria-current={isActive() ? "true" : undefined}
          onClick={props.onOpen}
        >
          <StateDot
            state={dotOfWorktree({
              queued: queued(),
              waiting: waiting(),
              working: working(),
              needs: needs(),
              live: live(),
            })}
          />
          <span class="name">{w().is_main ? mainBranch() : label()}</span>
          <Show when={!w().is_main && props.topic?.title && props.topic.title !== label()}>
            <span class="wt-topic" title={props.topic!.title}>
              {props.topic!.title}
            </span>
          </Show>
          <Show when={props.topic?.archived}>
            <Badge variant="quiet" title={T.archivedTip}>
              {T.archived}
            </Badge>
          </Show>
          <Show when={w().is_main && w().branch !== mainBranch()}>
            <span class="off-main" title={T.offMain(label(), mainBranch())}>
              <Icon name="warning" size={12} />
            </span>
          </Show>
          <Show when={st()?.dirty}>
            <Badge variant="quiet" title={T.dirtyTip(st()!.dirty)}>
              <i class="size-1.5 flex-none rounded-full bg-current opacity-70" />
              {st()!.dirty}
            </Badge>
          </Show>
          <Show when={props.overlap}>
            {(t) => (
              <Badge variant="overlap" role="img" title={t()} aria-label={t()}>
                <Icon name="warning" size={11} />
                {props.overlapCount}
              </Badge>
            )}
          </Show>
          <Show when={st()?.ahead || st()?.behind}>
            <Badge variant="quiet" title={T.aheadBehind(st()!.ahead, st()!.behind)}>
              {st()!.ahead ? (
                <span class="inline-flex items-center gap-px">
                  <Icon name="arrowUp" size={10} />
                  {st()!.ahead}
                </span>
              ) : null}
              {st()!.behind ? (
                <span class="inline-flex items-center gap-px">
                  <Icon name="arrowDown" size={10} />
                  {st()!.behind}
                </span>
              ) : null}
            </Badge>
          </Show>
          <Show when={pr()}>
            <Badge
              variant={
                pr()!.checks === "pass"
                  ? "prOk"
                  : pr()!.checks === "fail"
                    ? "prErr"
                    : pr()!.checks === "pending"
                      ? "prPend"
                      : "pr"
              }
              title={T.prTip(pr()!.number, pr()!.title, pr()!.checks)}
            >
              #{pr()!.number}
              {pr()!.checks === "pass" ? (
                <Icon name="check" size={10} inline />
              ) : pr()!.checks === "fail" ? (
                <Icon name="x" size={10} inline />
              ) : pr()!.checks === "pending" ? (
                <Icon name="ellipsis" size={10} inline />
              ) : null}
            </Badge>
          </Show>
          <Show
            when={props.folded && props.groupLive > 0}
            fallback={
              <Show when={live() > 0}>
                <span class="count">{live()}</span>
              </Show>
            }
          >
            <span class="count group" title={T.groupLive(props.groupLive, props.kids)}>
              {props.groupLive}
            </span>
          </Show>
          <Show when={isPinned(w().path)}>
            <span class="pin" title={T.pinned}>
              <Icon name="pin" size={11} />
            </span>
          </Show>
        </Button>
        <Button
          variant="icon"
          size="glyph"
          class="x"
          title={T.moreActions}
          aria-label={T.moreActions}
          onClick={(e) => {
            e.stopPropagation();
            const r = e.currentTarget.getBoundingClientRect();
            showMenuAt(r.left, r.bottom + 2, props.menu(label()));
          }}
        >
          <Icon name="more" />
        </Button>
        <Show when={!w().is_main}>
          <Button
            variant="icon"
            size="glyph"
            class="x"
            title={T.removeWorktree}
            aria-label={T.removeWorktree}
            onClick={(e) => {
              e.stopPropagation();
              props.onDrop(label());
            }}
          >
            <Icon name="x" />
          </Button>
        </Show>
      </div>
      <AgentList
        path={w().path}
        depth={props.depth}
        hidden={props.agentsHidden}
        onToggle={props.onToggleAgents}
        isMain={w().is_main}
        onStartLead={props.onStartLead}
      />
    </>
  );
}

/** A finished (integrated) worktree in the "Concluídos" group. */
export function FinishedRow(props: {
  wt: Wt;
  menu: (label: string) => MenuItem[];
  onOpen: () => void;
  onDrop: (label: string) => void;
}) {
  const label = () => props.wt.branch ?? P.sidebar.detached;
  const active = () => norm(selectedWorktree()) === norm(props.wt.path);
  return (
    <div
      class="item wt finished"
      classList={{ active: active() }}
      onContextMenu={(e) => showMenu(e, props.menu(label()))}
      title={props.wt.path}
    >
      <Button
        type="button"
        variant="row"
        size="bare"
        class="min-w-0 flex-1 gap-2"
        aria-current={active() ? "true" : undefined}
        onClick={props.onOpen}
      >
        <StateDot state="done" />
        <span class="name">{label()}</span>
      </Button>
      <Button
        variant="icon"
        size="glyph"
        class="x"
        title={T.removeWorktree}
        aria-label={T.removeNamed(label())}
        onClick={(e) => {
          e.stopPropagation();
          props.onDrop(label());
        }}
      >
        <Icon name="x" />
      </Button>
    </div>
  );
}
