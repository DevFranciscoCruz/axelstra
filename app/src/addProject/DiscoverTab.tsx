import { Match, Show, Switch, createResource, createSignal } from "solid-js";
import { type Discovered, projectAddDiscovered, projectDiscover } from "../api";
import { errorText } from "../appError";
import { Icon } from "../Icon";
import {
  navStamp,
  projects,
  refreshProjects,
  refreshSessions,
  selectIfStill,
  setError,
  setShowAddProject,
  showNotice,
} from "../store";
import { AP } from "../strings.sources";
import { Badge, Button } from "../ui";
import { PickList, createPickNav } from "./PickList";

/** Git repositories already on disk that are not projects yet: arrows to move, Enter or a click adds one. */
export function DiscoverTab() {
  const [items, { refetch }] = createResource<{ list: Discovered[]; failed?: string }>(() =>
    projectDiscover().then(
      (list) => ({ list }),
      (e) => ({ list: [], failed: errorText(e) }),
    ),
  );
  const [adding, setAdding] = createSignal("");
  const list = () => items()?.list ?? [];

  const add = async (d: Discovered) => {
    if (adding()) return;
    setAdding(d.path);
    try {
      const stamp = navStamp();
      const p = await projectAddDiscovered(d.path);
      await refreshProjects();
      const main = projects()
        .find((x) => x.id === p.id)
        ?.worktrees.find((w) => w.is_main);
      if (main) selectIfStill(stamp, main.path);
      void refreshSessions();
      showNotice(AP.dsAdded(p.name));
      setShowAddProject(false);
    } catch (e) {
      setError(errorText(e));
      setAdding("");
    }
  };
  const nav = createPickNav<Discovered>(list, (d) => void add(d));

  return (
    <div class="flex flex-col gap-2">
      <Switch>
        <Match when={items.loading}>
          <div class="auto-note" role="status">
            {AP.dsLoading}
          </div>
        </Match>
        <Match when={items()?.failed}>
          <div role="alert" class="text-[var(--err)]">
            {items()!.failed}
          </div>
        </Match>
        <Match when={list().length === 0}>
          <div class="auto-note" role="status">
            {AP.dsNone}
          </div>
        </Match>
        <Match when={true}>
          <div role="status" aria-live="polite" class="auto-note">
            {AP.dsCount(list().length)}
          </div>
          <PickList
            id="ap-ds-list"
            label={AP.dsList}
            items={list()}
            nav={nav}
            disabled={!!adding()}
            onPick={(d) => void add(d)}
          >
            {(d) => (
              <>
                <span class="flex items-center gap-2">
                  <Icon name="folder" size={13} />
                  <span class="truncate font-medium">{d.name}</span>
                  <Show when={d.branch}>
                    <Badge>{d.branch}</Badge>
                  </Show>
                  <Badge>{d.source === "recent" ? AP.dsRecent : AP.dsScan}</Badge>
                  <Show when={adding() === d.path}>
                    <span class="auto-note">{AP.dsAdding}</span>
                  </Show>
                </span>
                <span class="truncate text-xs text-muted-foreground">{d.path}</span>
              </>
            )}
          </PickList>
          <div class="auto-note">{AP.dsHint}</div>
        </Match>
      </Switch>
      <div class="m-actions">
        <Button type="button" disabled={items.loading || !!adding()} onClick={() => void refetch()}>
          {AP.dsRefresh}
        </Button>
      </div>
    </div>
  );
}
