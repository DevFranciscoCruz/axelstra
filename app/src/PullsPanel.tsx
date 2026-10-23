import { openUrl } from "@tauri-apps/plugin-opener";
import { Icon } from "./Icon";
import { For, Show, createResource, createSignal } from "solid-js";
import { GhItem, ghList, worktreeFromPr } from "./api";
import { RightTabs } from "./RightTabs";
import { findWorktree, refreshProjects, selectWorktree, selectedWorktree, setError } from "./store";

const CHECK: Record<string, [string, string]> = { pass: ["check", "Checks a passar"], fail: ["x", "Checks a falhar"], pending: ["ellipsis", "Checks a correr"] };
const REVIEW: Record<string, string> = { approved: "aprovado", changes: "alterações pedidas", review: "falta revisão" };

/** Open pull requests and issues of the selected project, read through the GitHub CLI. */
export function PullsPanel() {
  const [kind, setKind] = createSignal<"pr" | "issue">("pr");
  const project = () => findWorktree(selectedWorktree())?.project.id ?? null;
  const [items, { refetch }] = createResource(
    () => ({ p: project(), k: kind() }),
    async ({ p, k }): Promise<GhItem[]> => (p ? ghList(p, k) : []),
  );
  const [busy, setBusy] = createSignal<number | null>(null);

  const open = async (url: string) => {
    try {
      await openUrl(url);
    } catch (e) {
      setError(String(e));
    }
  };

  // a worktree holding the PR's head, ready for an agent to review or fix it
  const work = async (n: number) => {
    const p = project();
    if (!p) return;
    setBusy(n);
    try {
      const w = await worktreeFromPr(p, n);
      await refreshProjects();
      selectWorktree(w.path);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <aside class="rightbar">
      <RightTabs />
      <div class="rtools">
        <button class="icon" classList={{ on: kind() === "pr" }} onClick={() => setKind("pr")}>
          PRs
        </button>
        <button class="icon" classList={{ on: kind() === "issue" }} onClick={() => setKind("issue")}>
          Issues
        </button>
        <span class="grow" />
        <button class="icon" title="Atualizar" aria-label="Atualizar" onClick={() => void refetch()}>
          <Icon name="refresh" />
        </button>
      </div>
      <div class="rlist">
        <Show when={!items.loading} fallback={<div class="placeholder">A carregar…</div>}>
          <Show when={!items.error} fallback={<div class="placeholder">{String(items.error?.message ?? items.error)}</div>}>
            <For each={items()} fallback={<div class="placeholder">Nada em aberto</div>}>
              {(i) => (
                <div class="pr">
                  <div class="pr-title">
                    <Show when={i.checks}>
                      <span class={`chk ${i.checks}`} title={CHECK[i.checks][1]}>
                        <Icon name={CHECK[i.checks][0]} size={12} />
                      </span>
                    </Show>
                    <span class="pr-num">#{i.number}</span>
                    <span class="pr-text" title={i.title}>
                      {i.title}
                    </span>
                  </div>
                  <div class="pr-meta">
                    <span>{i.author}</span>
                    <Show when={i.draft}>
                      <span class="chip">rascunho</span>
                    </Show>
                    <Show when={i.review}>
                      <span class="chip">{REVIEW[i.review]}</span>
                    </Show>
                    <Show when={i.branch}>
                      <span class="chip mono">{i.branch}</span>
                    </Show>
                    <span class="grow" />
                    <Show when={kind() === "pr"}>
                      <button class="icon" disabled={busy() === i.number} title="Abrir num worktree novo" onClick={() => void work(i.number)}>
                        {busy() === i.number ? "…" : "Worktree"}
                      </button>
                    </Show>
                    <button class="icon" title="Abrir no navegador" aria-label="Abrir no navegador" onClick={() => void open(i.url)}>
                      <Icon name="external" />
                    </button>
                  </div>
                </div>
              )}
            </For>
          </Show>
        </Show>
      </div>
    </aside>
  );
}
