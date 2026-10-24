import { For, Show, createResource } from "solid-js";
import { envTools, type EnvTool } from "./api";
import { Button } from "./ui";
import { Icon } from "./Icon";
import { K } from "./keyboard";
import { commandById } from "./commands";
import { W } from "./strings.welcome";
import logo from "./assets/logo.png";

const needs = [W.needGit, W.needAgent, W.needGh];
const toolsNow = (): Promise<EnvTool[] | null> => {
  try {
    return envTools().catch(() => null);
  } catch {
    return Promise.resolve(null);
  }
};
type Wt = { path: string; branch: string | null; is_main: boolean };

const Kbd = (props: { children: string }) => (
  <kbd class="rounded-sm border border-border bg-muted px-1.5 py-0.5 text-xs text-foreground">{props.children}</kbd>
);

/**
 * The main area with nothing open. Without `worktrees`: no project yet, one button to add it.
 * With `worktrees`: a project exists but no worktree is selected; they are listed to pick, or a new one is made.
 */
export function Welcome(props: {
  onAddProject: () => void;
  worktrees?: Wt[];
  projectName?: string;
  onPick?: (path: string) => void;
  onNewWorktree?: () => void;
}) {
  const paletteKeys = () => K(commandById("palette")?.keys ?? "Ctrl+K");
  // what the daemon finds installed; without an answer (old daemon, no host) the hints stay as plain advice
  const [tools] = createResource(toolsNow);
  const toolOf = (id: string) => tools()?.find((t) => t.id === id);
  const shortcutKeys = () => commandById("shortcuts")?.keys ?? "?";
  return (
    <section
      class="mx-auto flex h-full w-full max-w-[420px] flex-col items-center justify-center gap-5 p-8 text-center"
      aria-labelledby="welcome-title"
    >
      <img src={logo} alt="" class="size-14" />
      <div class="flex flex-col gap-2">
        <h1 id="welcome-title" class="text-2xl font-semibold text-foreground">
          {props.worktrees ? W.pickTitle : W.title}
        </h1>
        <p class="text-base text-muted-foreground">{props.worktrees ? W.pickFlow(props.projectName) : W.flow}</p>
      </div>
      <Show
        when={props.worktrees}
        fallback={
          <Button variant="default" class="gap-2" onClick={props.onAddProject}>
            <Icon name="plus" size={14} />
            {W.add}
          </Button>
        }
      >
        {(wts) => (
          <>
            <ul class="flex w-full flex-col gap-1 text-left">
              <For each={wts()}>
                {(w) => (
                  <li>
                    <Button
                      type="button"
                      variant="row"
                      size="bare"
                      class="w-full gap-2 px-2 py-1.5"
                      onClick={() => props.onPick?.(w.path)}
                      title={w.path}
                    >
                      <Icon name="branch" size={13} />
                      <span class="min-w-0 flex-1 truncate">{w.branch ?? W.detached}</span>
                      <Show when={w.is_main}>
                        <span class="text-xs text-muted-foreground">{W.mainTag}</span>
                      </Show>
                    </Button>
                  </li>
                )}
              </For>
            </ul>
            <Button variant="default" class="gap-2" onClick={() => props.onNewWorktree?.()}>
              <Icon name="plus" size={14} />
              {W.newWorktree}
            </Button>
          </>
        )}
      </Show>
      <p class="flex flex-wrap items-center justify-center gap-x-2 gap-y-1 text-sm text-muted-foreground">
        <Kbd>{paletteKeys()}</Kbd> {W.palette} · <Kbd>{shortcutKeys()}</Kbd> {W.shortcuts}
      </p>
      <Show when={!props.worktrees}>
        <div class="text-left text-sm text-muted-foreground">
          <div class="mb-1 font-semibold text-foreground">{W.needs}</div>
          <ul class="flex flex-col gap-1">
            <For each={needs}>
              {(n) => (
                <li data-tool-state={toolOf(n.name) ? (toolOf(n.name)!.found ? "found" : "missing") : "unknown"}>
                  <code class="text-foreground">{n.name}</code>:{" "}
                  <Show when={toolOf(n.name)} fallback={n.why}>
                    {(t) =>
                      t().found ? `${W.toolFound}${t().version ? ` ${t().version}` : ""}` : `${W.toolMissing}, ${n.why}`
                    }
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </div>
      </Show>
    </section>
  );
}
