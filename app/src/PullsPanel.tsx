import { Loading } from "./Loading";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Badge, Button } from "./ui";
import { Icon } from "./Icon";
import { For, Show, createResource, createSignal } from "solid-js";
import { GhItem, ghList, worktreeFromPr } from "./api";
import { RightTabs } from "./RightTabs";
import { findWorktree, refreshProjects, selectWorktree, selectedWorktree, setError, rightTab , showNotice } from "./store";

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
      showNotice(`Worktree do PR #${n} criado.`);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <aside class="rightbar" id="rpanel" role="tabpanel" aria-labelledby={`rtab-${rightTab()}`}>
      <RightTabs />
      <div class="rtools">
        <Button variant="icon" size="icon" class={kind() === "pr" ? "text-primary" : undefined} aria-pressed={kind() === "pr"} onClick={() => setKind("pr")}>
          PRs
        </Button>
        <Button variant="icon" size="icon" class={kind() === "issue" ? "text-primary" : undefined} aria-pressed={kind() === "issue"} onClick={() => setKind("issue")}>
          Problemas
        </Button>
        <span class="grow" />
        <Button variant="icon" size="icon" title="Atualizar" aria-label="Atualizar" onClick={() => void refetch()}>
          <Icon name="refresh" />
        </Button>
      </div>
      <div class="rlist">
        <Show when={!items.loading} fallback={<Loading onRetry={() => void refetch()} />}>
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
                      <Badge>rascunho</Badge>
                    </Show>
                    <Show when={i.review}>
                      <Badge>{REVIEW[i.review]}</Badge>
                    </Show>
                    <Show when={i.branch}>
                      <Badge class="mono">{i.branch}</Badge>
                    </Show>
                    <span class="grow" />
                    <Show when={kind() === "pr"}>
                      <Button variant="icon" size="icon" disabled={busy() === i.number} title="Abrir num worktree novo" onClick={() => void work(i.number)}>
                        {busy() === i.number ? "…" : "Worktree"}
                      </Button>
                    </Show>
                    <Button variant="icon" size="icon" title="Abrir no navegador" aria-label="Abrir no navegador" onClick={() => void open(i.url)}>
                      <Icon name="external" />
                    </Button>
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
