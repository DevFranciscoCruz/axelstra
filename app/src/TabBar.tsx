import { For, Show, createEffect, createSignal, on, onCleanup, onMount } from "solid-js";
import { SessionInfo } from "./api";
import { DOT_LABEL, dotOfSession, dotTip } from "./agentState";
import { MenuItem, showMenu, showMenuAt } from "./ContextMenu";
import { tabMoveTarget } from "./layout";
import { Icon } from "./Icon";
import { basename } from "./paths";
import { StateDot } from "./StateDot";
import { Tab } from "./Tab";
import { S } from "./strings";
import { Button, Input, Tabs, TabsList } from "./ui";
import { activeSession, guarded, moveTab, renameTab, setView, showView, tabLabel, view } from "./store";

type Props = {
  tabs: SessionInfo[];
  renaming: number | null;
  setRenaming: (id: number | null) => void;
  showTab: (s: SessionInfo) => void;
  close: (s: SessionInfo) => void | Promise<void>;
};

/** the strip of tabs: Kanban, Docker, one per terminal (rename, drag to reorder, middle-click closes) and the open commit/file/diff */
export function TabBar(props: Props) {
  // tab being dragged and the one it hovers
  const [dragTab, setDragTab] = createSignal<number | null>(null);
  const [dropOn, setDropOn] = createSignal<number | null>(null);
  // the one tab Kobalte treats as selected (it keeps the roving tabindex there); each Tab also states its own `selected`
  const stripValue = () =>
    view().kind !== "terminal" ? view().kind : activeSession() !== null ? `tab-${activeSession()}` : "";
  // renaming ends with the new name (null = cancelled); focus goes back to the tab, not to the page
  const finishRename = (id: number, name: string | null, refocus = true) => {
    if (name !== null) renameTab(id, name);
    props.setRenaming(null);
    if (refocus) queueMicrotask(() => document.querySelector<HTMLElement>(`[data-tab="tab-${id}"]`)?.focus());
  };
  // the commit / file / diff view is a tab of its own while it is the one showing
  // reorder one place; focus stays on the moved tab
  const moveBy = (s: SessionInfo, dir: -1 | 1) => {
    const ids = props.tabs.map((t) => t.id);
    const to = tabMoveTarget(ids, s.id, dir);
    if (to === undefined) return;
    moveTab(ids, s.id, to);
    queueMicrotask(() => document.querySelector<HTMLElement>(`[data-tab="tab-${s.id}"]`)?.focus());
  };
  const tabMenu = (s: SessionInfo): MenuItem[] => {
    const ids = props.tabs.map((t) => t.id);
    return [
      { label: S.app.tabRename, run: () => props.setRenaming(s.id) },
      { label: S.app.tabMoveLeft, disabled: tabMoveTarget(ids, s.id, -1) === undefined, run: () => moveBy(s, -1) },
      { label: S.app.tabMoveRight, disabled: tabMoveTarget(ids, s.id, 1) === undefined, run: () => moveBy(s, 1) },
      { separator: true },
      {
        label: s.meta?.kind === "agent" ? S.app.closeAgent : S.app.closeTerminal,
        danger: true,
        run: () => void props.close(s),
      },
    ];
  };

  // overflow of the strip: cues on the sides with more tabs, the active tab kept in view, wheel scrolls sideways
  let list: HTMLElement | undefined;
  const [moreLeft, setMoreLeft] = createSignal(false);
  const [moreRight, setMoreRight] = createSignal(false);
  const measure = () => {
    if (!list) return;
    setMoreLeft(list.scrollLeft > 1);
    setMoreRight(list.scrollLeft + list.clientWidth < list.scrollWidth - 1);
  };
  const reveal = () =>
    requestAnimationFrame(() => {
      list?.querySelector<HTMLElement>("[data-active]")?.scrollIntoView({ inline: "nearest", block: "nearest" });
      measure();
    });
  onMount(() => {
    if (!list) return;
    const ro = new ResizeObserver(measure);
    ro.observe(list);
    // tabs coming and going change the scroll width without resizing the strip
    const mo = new MutationObserver(measure);
    mo.observe(list, { childList: true, subtree: true });
    onCleanup(() => {
      ro.disconnect();
      mo.disconnect();
    });
    reveal();
  });
  createEffect(on([() => view().kind, activeSession, () => props.tabs.length], reveal, { defer: true }));
  const onWheel = (e: WheelEvent) => {
    if (!list || list.scrollWidth <= list.clientWidth || Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
    e.preventDefault();
    list.scrollLeft += e.deltaY;
  };
  const overflowing = () => moreLeft() || moreRight();
  const listMenu = (e: MouseEvent & { currentTarget: HTMLElement }) => {
    const items: MenuItem[] = [
      { label: "Kanban", run: () => guarded(() => setView({ kind: "kanban" })) },
      { label: "Docker", run: () => guarded(() => setView({ kind: "docker" })) },
      ...props.tabs.map((s) => ({
        label: `${tabLabel(s)} · ${DOT_LABEL[dotOfSession(s)]}`,
        run: () => props.showTab(s),
      })),
    ];
    const r = e.currentTarget.getBoundingClientRect();
    showMenuAt(r.left, r.bottom + 2, items);
  };

  const shown = <K extends "commit" | "file" | "diff">(k: K) => {
    const v = view();
    return v.kind === k ? (v as Extract<typeof v, { kind: K }>) : null;
  };

  return (
    <>
      <Tabs
        value={stripValue()}
        onChange={() => {}}
        activationMode="manual"
        class="tabstrip flex min-w-0"
        data-more-left={moreLeft() ? "" : undefined}
        data-more-right={moreRight() ? "" : undefined}
      >
        <TabsList
          ref={(el: HTMLElement) => (list = el)}
          onScroll={measure}
          onWheel={onWheel}
          aria-label={S.app.tabs}
          class="min-w-0 overflow-x-auto [scrollbar-width:none]"
        >
          <Tab
            value="kanban"
            selected={view().kind === "kanban"}
            onSelect={() => guarded(() => setView({ kind: "kanban" }))}
          >
            <span class="dtag">
              <Icon name="grid" size={11} />
            </span>
            <span>Kanban</span>
          </Tab>
          <Tab
            value="docker"
            selected={view().kind === "docker"}
            onSelect={() => guarded(() => setView({ kind: "docker" }))}
          >
            <span class="dtag">
              <Icon name="box" size={11} />
            </span>
            <span>Docker</span>
          </Tab>
          <For each={props.tabs}>
            {(s) => (
              <Tab
                id={`tab-${s.id}`}
                value={`tab-${s.id}`}
                selected={view().kind === "terminal" && activeSession() === s.id}
                onSelect={() => props.showTab(s)}
                onClose={() => void props.close(s)}
                onRename={() => props.setRenaming(s.id)}
                onMove={(dir) => moveBy(s, dir)}
                dropBefore={dropOn() === s.id && dragTab() !== s.id}
                wrap={{
                  draggable: props.renaming !== s.id,
                  onDragStart: (e) => {
                    setDragTab(s.id);
                    e.dataTransfer?.setData("text/plain", String(s.id));
                  },
                  onDragOver: (e) => {
                    if (dragTab() === null) return;
                    e.preventDefault();
                    setDropOn(s.id);
                  },
                  onDrop: (e) => {
                    e.preventDefault();
                    const from = dragTab();
                    if (from !== null)
                      moveTab(
                        props.tabs.map((t) => t.id),
                        from,
                        s.id,
                      );
                    setDragTab(null);
                    setDropOn(null);
                  },
                  onDragEnd: () => {
                    setDragTab(null);
                    setDropOn(null);
                  },
                  onDblClick: () => props.setRenaming(s.id),
                  onContextMenu: (e) => showMenu(e, tabMenu(s)),
                  onMouseDown: (e) => e.button === 1 && e.preventDefault(),
                  onAuxClick: (e) => {
                    if (e.button !== 1) return;
                    e.preventDefault();
                    void props.close(s);
                  },
                }}
                editing={
                  props.renaming === s.id ? (
                    <>
                      <StateDot state={dotOfSession(s)} tip={dotTip(s)} />
                      <Input
                        class="h-auto w-[120px] rounded-sm border-border bg-transparent px-1 py-0 text-inherit"
                        aria-label={S.app.tabName}
                        value={tabLabel(s)}
                        onClick={(e) => e.stopPropagation()}
                        onKeyDown={(e) => {
                          e.stopPropagation();
                          if (e.key === "Enter") finishRename(s.id, e.currentTarget.value);
                          else if (e.key === "Escape") finishRename(s.id, null);
                        }}
                        onBlur={(e) => props.renaming === s.id && finishRename(s.id, e.currentTarget.value, false)}
                        ref={(el) =>
                          queueMicrotask(() => {
                            el.focus();
                            el.select();
                          })
                        }
                      />
                    </>
                  ) : undefined
                }
              >
                <StateDot state={dotOfSession(s)} tip={dotTip(s)} />
                <span>
                  {s.meta?.worker ? <Icon name="subArrow" size={11} inline /> : null}
                  {tabLabel(s)}
                </span>
              </Tab>
            )}
          </For>
          <Show when={shown("commit")}>
            {(c) => (
              <Tab
                value="commit"
                class="diff-tab"
                selected
                title={c().commit.subject}
                onSelect={() => setView(c())}
                onClose={() => showView()}
                closeLabel={S.app.closeCommit}
              >
                <span class="dtag">
                  <Icon name="branch" size={11} />
                </span>
                <span>{c().commit.short}</span>
              </Tab>
            )}
          </Show>
          <Show when={shown("file")}>
            {(p) => (
              <Tab
                value="file"
                class="diff-tab"
                selected
                title={p().path}
                onSelect={() => setView(p())}
                onClose={() => guarded(() => showView())}
                closeLabel={S.app.closeFile}
              >
                <span class="dtag">
                  <Icon name="pencil" size={11} />
                </span>
                <span>{basename(p().path)}</span>
              </Tab>
            )}
          </Show>
          <Show when={shown("diff")}>
            {(p) => (
              <Tab
                value="diff"
                class="diff-tab"
                selected
                title={p().path}
                onSelect={() => setView(p())}
                onClose={() => showView()}
                closeLabel={S.app.closeDiff}
              >
                <span class="dtag">Δ</span>
                <span>{basename(p().path)}</span>
              </Tab>
            )}
          </Show>
        </TabsList>
      </Tabs>
      <Show when={overflowing()}>
        <Button
          variant="icon"
          size="tool"
          class="self-center"
          title={S.app.tabList}
          aria-label={S.app.tabList}
          aria-haspopup="menu"
          onClick={listMenu}
        >
          <Icon name="chevDown" />
        </Button>
      </Show>
    </>
  );
}
