import { Match, Show, Switch, createMemo, createResource, createSignal } from "solid-js";
import { Badge, Button, Input } from "../ui";
import { type GhRepo, type GhRepos, ghRepos } from "../api";
import { errorText } from "../appError";
import { AP } from "../strings.sources";
import { CloneProgress } from "./CloneProgress";
import { FolderField } from "./FolderField";
import { cloneActive, startClone } from "./cloneJob";
import { PickList, createPickNav } from "./PickList";
import { filterRepos, joinPath, lastParent, rememberParent, repoNameFromUrl } from "./logic";

/** Your GitHub repositories through `gh` (its login, never a token of ours): filter, move with the arrows, Enter or a click clones. */
export function GithubTab() {
  const [parent, setParent] = createSignal(lastParent());
  const [parentError, setParentError] = createSignal("");
  const [query, setQuery] = createSignal("");
  const [data, { refetch }] = createResource<GhRepos & { failed?: string }>(() =>
    ghRepos().catch((e) => ({ state: "ok" as const, repos: [], failed: errorText(e) })),
  );
  const visible = createMemo(() => filterRepos(data()?.repos ?? [], query()));
  const busy = () => cloneActive();

  const clone = (r: GhRepo) => {
    if (busy()) return;
    if (!parent().trim()) {
      setParentError(AP.folderRequired);
      return;
    }
    setParentError("");
    rememberParent(parent());
    const name = repoNameFromUrl(r.url) || r.name_with_owner.split("/").pop() || r.name_with_owner;
    void startClone({ url: r.url, dest: joinPath(parent(), name) });
  };
  const nav = createPickNav<GhRepo>(visible, clone);

  return (
    <div class="flex flex-col gap-2">
      <FolderField
        id="ap-gh-parent"
        label={AP.folder}
        value={parent()}
        onChange={(v) => {
          setParent(v);
          setParentError("");
        }}
        error={parentError()}
        disabled={busy()}
      />
      <Switch>
        <Match when={data.loading}>
          <div class="auto-note" role="status">
            {AP.ghLoading}
          </div>
        </Match>
        <Match when={data()?.failed}>
          <div role="alert" class="flex flex-col gap-2 text-[var(--err)]">
            <span>{AP.ghFailed(data()!.failed!)}</span>
            <Retry onClick={() => void refetch()} />
          </div>
        </Match>
        <Match when={data()?.state === "no_gh"}>
          <div role="alert" class="flex flex-col gap-2">
            <span>{AP.ghNoGh}</span>
            <Retry onClick={() => void refetch()} />
          </div>
        </Match>
        <Match when={data()?.state === "not_authed"}>
          <div role="alert" class="flex flex-col gap-2">
            <span>{AP.ghNotAuthed}</span>
            <Retry onClick={() => void refetch()} />
          </div>
        </Match>
        <Match when={data()}>
          <Show when={(data()?.repos.length ?? 0) > 0} fallback={<div class="auto-note">{AP.ghNone}</div>}>
            <label class="flex flex-col gap-1 text-xs text-muted-foreground">
              {AP.filter}
              <Input
                role="combobox"
                aria-expanded="true"
                aria-controls="ap-gh-list"
                aria-activedescendant={nav.row() >= 0 ? `ap-gh-list-${nav.row()}` : undefined}
                value={query()}
                placeholder={AP.filterPh}
                autocomplete="off"
                spellcheck={false}
                onInput={(e) => setQuery(e.currentTarget.value)}
                onKeyDown={(e) => {
                  // Home/End stay with the caret of the filter
                  if (!["Home", "End"].includes(e.key) && nav.onKey(e)) e.preventDefault();
                }}
              />
            </label>
            <div role="status" aria-live="polite" class="auto-note">
              {AP.ghCount(visible().length)}
            </div>
            <Show when={visible().length > 0} fallback={<div class="auto-note">{AP.ghEmpty}</div>}>
              <PickList id="ap-gh-list" label={AP.ghList} items={visible()} nav={nav} disabled={busy()} onPick={clone}>
                {(r) => (
                  <>
                    <span class="flex items-center gap-2">
                      <span class="truncate font-medium" title={AP.ghCloneTip(r.name_with_owner)}>
                        {r.name_with_owner}
                      </span>
                      <Show when={r.is_private}>
                        <Badge>{AP.ghPrivate}</Badge>
                      </Show>
                    </span>
                    <Show when={r.description}>
                      <span class="truncate text-xs text-muted-foreground">{r.description}</span>
                    </Show>
                  </>
                )}
              </PickList>
              <div class="auto-note">{AP.ghHint}</div>
            </Show>
          </Show>
        </Match>
      </Switch>
      <CloneProgress />
    </div>
  );
}

function Retry(props: { onClick: () => void }) {
  return (
    <div class="m-actions">
      <Button type="button" data-ap-primary="" onClick={props.onClick}>
        {AP.ghRetry}
      </Button>
    </div>
  );
}
