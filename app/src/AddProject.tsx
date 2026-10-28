import { For, Match, Show, Switch } from "solid-js";
import { AddRemoteForm } from "./AddRemoteProject";
import { Icon } from "./Icon";
import { Modal } from "./Modal";
import { CloneTab } from "./addProject/CloneTab";
import { DiscoverTab } from "./addProject/DiscoverTab";
import { GithubTab } from "./addProject/GithubTab";
import { LocalTab } from "./addProject/LocalTab";
import { NewTab } from "./addProject/NewTab";
import { SOURCES, type Source, isSource, lastSource, rememberSource } from "./addProject/logic";
import { addProjectTab, setAddProjectTab, setShowAddProject } from "./store";
import { S } from "./strings";
import { AP } from "./strings.sources";
import { Button, Tabs, TabsList, TabsTrigger } from "./ui";
import { addProjectDirty } from "./addProject/dirty";

/*
 * UX decision (Add project): ONE dialog with a vertical list of origins on the left and the form of the chosen origin on the
 * right, instead of a first screen of big buttons and a second step. Why: the origins are peers, a user who clones every day
 * should be one keystroke from it, and the dialog remembers the last origin (localStorage, best effort) and the last parent
 * folder of a clone. The list is a real tablist (Kobalte): arrows move AND select, Home/End jump, Enter on a tab runs the
 * primary action of the panel (the button marked data-ap-primary), Esc closes (Dialog), Tab is trapped by Dialog.
 * Progress and errors sit in aria-live regions; fields are disabled while a job runs.
 * Limits, on purpose: the page never names a path for project.add (see api.ts), so a folder dropped on the "Pasta local"
 * zone only tells which folder to pick in the native dialog; worktree.from_pr is not here, it needs a project that is
 * already added and lives in the sidebar. Tokens are never asked for nor stored: clone and GitHub use the user's git / gh login.
 */

const ICON: Record<Source, string> = {
  local: "folder",
  clone: "link",
  github: "branch",
  new: "plus",
  remote: "external",
  discover: "sparkle",
};

/** The one way in for a new project, whatever its origin. */
export function AddProject() {
  const close = () => setShowAddProject(false);
  const active = (): Source => {
    const t = addProjectTab();
    return isSource(t) ? t : lastSource();
  };
  const choose = (v: string) => {
    if (!isSource(v)) return;
    rememberSource(v);
    setAddProjectTab(v);
  };
  let panel: HTMLDivElement | undefined;
  /** Enter on the list of origins (or inside a single-line field that did not use it) runs the panel's main button */
  const primary = () => panel?.querySelector<HTMLButtonElement>("[data-ap-primary]:not(:disabled)")?.click();

  return (
    <Modal label={AP.title} class="modal add-project" dirty={addProjectDirty()} onClose={close}>
      <div class="auto-head">
        <strong>{AP.title}</strong>
        <span class="grow" />
        <Button onClick={close}>{S.close}</Button>
      </div>
      <Tabs orientation="vertical" value={active()} onChange={choose} class="flex min-h-80 flex-col gap-3 sm:flex-row">
        <TabsList
          aria-label={AP.sources}
          class="flex shrink-0 flex-row flex-wrap gap-1 sm:w-48 sm:flex-col"
          onKeyDown={(e: KeyboardEvent) => {
            if (e.key === "Enter") {
              e.preventDefault();
              primary();
            }
          }}
        >
          <For each={SOURCES}>
            {(s) => (
              <TabsTrigger
                value={s}
                variant="bar"
                class="flex w-full flex-col items-start gap-0.5 rounded-md px-2.5 py-2 text-left hover:bg-accent data-[selected]:bg-accent"
              >
                <span class="flex items-center gap-2 text-base font-medium">
                  <Icon name={ICON[s]} size={14} />
                  {AP.tab[s]}
                </span>
                <span class="hidden text-xs text-muted-foreground sm:block">{AP.hint[s]}</span>
              </TabsTrigger>
            )}
          </For>
        </TabsList>
        <div ref={panel} class="min-w-0 flex-1">
          <For each={SOURCES}>
            {(s) => (
              <Tabs.Content value={s} class="flex flex-col gap-2 outline-none">
                <Show when={active() === s}>
                  <Switch>
                    <Match when={s === "local"}>
                      <LocalTab onClose={close} />
                    </Match>
                    <Match when={s === "clone"}>
                      <CloneTab />
                    </Match>
                    <Match when={s === "github"}>
                      <GithubTab />
                    </Match>
                    <Match when={s === "new"}>
                      <NewTab />
                    </Match>
                    <Match when={s === "remote"}>
                      <AddRemoteForm onClose={close} embedded />
                    </Match>
                    <Match when={s === "discover"}>
                      <DiscoverTab />
                    </Match>
                  </Switch>
                </Show>
              </Tabs.Content>
            )}
          </For>
        </div>
      </Tabs>
    </Modal>
  );
}
