import { RightTabs } from "./RightTabs";
import { ask } from "./Confirm";
import { For, Show, createResource, createSignal } from "solid-js";
import { Commit, FileChange, gitCommit, gitLog, gitPr, gitPush } from "./api";
import { showMenu } from "./ContextMenu";
import { press } from "./a11y";
import {
  guarded,
  baseRef,
  changes,
  changesMode,
  findWorktree,
  openCommit,
  openDiff,
  refreshChanges,
  selectedWorktree,
  setChangesMode,
  setOpenCommit,
  setOpenDiff,
  setOpenFile,
  setShowKanban,
} from "./store";

const LABEL: Record<string, string> = { M: "alterado", A: "novo", D: "apagado", U: "conflito" };

const split = (p: string) => {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i < 0 ? ["", p] : [p.slice(0, i), p.slice(i + 1)];
};

/** Branch the commits list is measured against: the project's default branch (none on the main worktree). */
const commitsBase = (): string | undefined => {
  const c = findWorktree(selectedWorktree());
  if (!c || c.wt.is_main) return undefined;
  return c.project.default_branch ?? c.project.worktrees.find((w) => w.is_main)?.branch ?? undefined;
};

const when = (iso: string) => iso.slice(0, 16).replace("T", " ");

/** Source control in the usual shape: branch, commit box, CHANGES, COMMITS. */
export function ChangesPanel() {
  const files = () => changes()?.files ?? [];
  const cur = () => findWorktree(selectedWorktree());
  const branch = () => cur()?.wt.branch ?? "";
  const base = () => (cur()?.wt.is_main ? "origin" : (commitsBase() ?? "main"));

  const [message, setMessage] = createSignal("");
  const [prOpen, setPrOpen] = createSignal(false);
  const [prTitle, setPrTitle] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [note, setNote] = createSignal<{ ok: boolean; text: string } | null>(null);
  const [showChanges, setShowChanges] = createSignal(true);
  const [showCommits, setShowCommits] = createSignal(true);

  const [log, { refetch: refetchLog }] = createResource(
    () => ({ wt: selectedWorktree(), base: commitsBase(), n: changes()?.files.length ?? 0 }),
    (k) => (k.wt ? gitLog(k.wt, k.base) : []),
  );

  const act = async (f: () => Promise<string>) => {
    setBusy(true);
    setNote(null);
    try {
      setNote({ ok: true, text: await f() });
      setTimeout(() => setNote((n) => (n?.ok ? null : n)), 6000);
    } catch (e) {
      setNote({ ok: false, text: String(e) });
    } finally {
      setBusy(false);
      void refreshChanges();
      void refetchLog();
    }
  };

  const commitOnly = async () => {
    const r = await gitCommit(selectedWorktree()!, message());
    setMessage("");
    return `Commit ${r.sha}`;
  };
  const doCommit = () => act(commitOnly);
  const confirmPush = () => ask(`Enviar o branch "${branch()}" para o remote origin?`, { title: "Push", kind: "warning" });
  const doPush = async () => {
    // pushing publishes the branch to the remote: always confirm
    if (!(await confirmPush())) return;
    await act(async () => {
      await gitPush(selectedWorktree()!);
      return `Push de ${branch()} feito`;
    });
  };
  const doCommitPush = async () => {
    if (!(await confirmPush())) return;
    await act(async () => {
      const c = await commitOnly();
      await gitPush(selectedWorktree()!);
      return `${c} e push de ${branch()}`;
    });
  };
  const doPr = async () => {
    if (!(await ask(`Criar um pull request de "${branch()}" no GitHub?`, { title: "Pull request", kind: "warning" }))) return;
    await act(async () => {
      const url = await gitPr(selectedWorktree()!, prTitle(), "", changesMode() === "main" ? baseRef() : undefined);
      setPrOpen(false);
      setPrTitle("");
      return url;
    });
  };

  const canCommit = () => !busy() && files().length > 0 && message().trim().length > 0;

  const openFileDiff = (f: FileChange) =>
    guarded(() => {
      setOpenFile(null);
      setShowKanban(false);
      setOpenCommit(null);
      setOpenDiff(f.path);
    });
  const openCommitView = (c: Commit) =>
    guarded(() => {
      setOpenDiff(null);
      setOpenFile(null);
      setShowKanban(false);
      setOpenCommit(c);
    });

  return (
    <aside class="rightbar">
      <RightTabs />

      <div class="sc-head">
        <button class="pill" disabled={busy()} onClick={() => setPrOpen(!prOpen())}>
          ⑂ Criar PR
        </button>
        <span class="grow" />
        <button
          class="icon"
          title="Atualizar"
          onClick={() => {
            void refreshChanges();
            void refetchLog();
          }}
        >
          ↻
        </button>
        <button
          class="icon"
          title="Mais ações"
          onClick={(e) =>
            showMenu(e, [
              {
                label: "Por consolidar (vs HEAD)",
                icon: changesMode() === "head" ? "✓" : "",
                run: () => {
                  setChangesMode("head");
                  void refreshChanges();
                },
              },
              {
                label: "Todo o branch (vs principal)",
                icon: changesMode() === "main" ? "✓" : "",
                run: () => {
                  setChangesMode("main");
                  void refreshChanges();
                },
              },
              { separator: true },
              { label: "Push", icon: "↑", run: () => doPush() },
            ])
          }
        >
          ⋯
        </button>
      </div>
      <div class="sc-branch" title={branch()}>
        <div class="sc-name">{branch() || "(detached)"}</div>
        <div class="sc-base">→ {base()}</div>
      </div>

      <Show when={prOpen()}>
        <div class="prow sc-pr">
          <input
            placeholder="Título do pull request"
            value={prTitle()}
            onInput={(e) => setPrTitle(e.currentTarget.value)}
            onKeyDown={(e) => e.key === "Enter" && prTitle().trim() && void doPr()}
            ref={(el) => queueMicrotask(() => el.focus())}
          />
          <button disabled={busy() || !prTitle().trim()} onClick={() => void doPr()}>
            Criar
          </button>
        </div>
      </Show>

      <div class="sc-commit">
        <textarea
          placeholder="Mensagem (Ctrl+Enter para fazer commit)"
          rows={3}
          value={message()}
          onInput={(e) => setMessage(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && e.ctrlKey && canCommit()) void doCommit();
          }}
        />
        <div class="sc-split">
          <button class="sc-main" disabled={!canCommit()} title={canCommit() ? "Commit (Ctrl+Enter)" : files().length === 0 ? "Sem alterações para commit" : message().trim() ? "A trabalhar…" : "Escreva uma mensagem"} onClick={() => void doCommit()}>
            ✓ Commit
          </button>
          <button
            class="sc-drop"
            disabled={busy()}
            title="Mais opções de commit"
            onClick={(e) =>
              showMenu(e, [
                { label: "Commit", icon: "✓", disabled: !canCommit(), run: () => doCommit() },
                { label: "Commit e Push", icon: "↑", disabled: !canCommit(), run: () => doCommitPush() },
                { label: "Push", icon: "↑", run: () => doPush() },
              ])
            }
          >
            ⌄
          </button>
        </div>
        <Show when={note()}>
          {(n) => (
            <div class="pnote" classList={{ bad: !n().ok }} title={n().text}>
              {n().text}
            </div>
          )}
        </Show>
      </div>

      <div class="sc-body">
        <div class="sc-title" {...press(() => setShowChanges(!showChanges()))} aria-expanded={showChanges()}>
          <span class="chev">{showChanges() ? "⌄" : "›"}</span>
          <span>ALTERAÇÕES</span>
          <span class="sc-count">{files().length}</span>
        </div>
        <Show when={showChanges()}>
          <div class="sc-files">
            <For each={files()} fallback={<div class="empty">Sem alterações</div>}>
              {(f: FileChange) => {
                const [dir, name] = split(f.path);
                return (
                  <div class="file" classList={{ active: openDiff() === f.path }} {...press(() => openFileDiff(f))} title={`${f.path} (${LABEL[f.status] ?? f.status})`}>
                    <span class="ficon">▤</span>
                    <span class="fname2">
                      <span class="fn">{name}</span>
                      <span class="fd">{dir}</span>
                    </span>
                    <Show when={!f.binary} fallback={<span class="fbin">bin</span>}>
                      <span class="nums">
                        <Show when={f.added > 0}>
                          <span class="add">+{f.added}</span>
                        </Show>
                        <Show when={f.removed > 0}>
                          <span class="del">−{f.removed}</span>
                        </Show>
                      </span>
                    </Show>
                    <span class={`st st-${f.status}`}>{f.status}</span>
                  </div>
                );
              }}
            </For>
          </div>
        </Show>

        <div class="sc-title" {...press(() => setShowCommits(!showCommits()))} aria-expanded={showCommits()}>
          <span class="chev">{showCommits() ? "⌄" : "›"}</span>
          <span>COMMITS</span>
          <span class="sc-count">{log()?.length ?? 0}</span>
        </div>
        <Show when={showCommits()}>
          <div class="sc-files">
            <Show when={log.error}>
              <div class="empty">{String(log.error)}</div>
            </Show>
            <For each={log() ?? []} fallback={<Show when={!log.loading}><div class="empty">{commitsBase() ? `Nada à frente de ${commitsBase()}` : "Sem commits"}</div></Show>}>
              {(c) => (
                <div class="commit" classList={{ active: openCommit()?.sha === c.sha }} {...press(() => openCommitView(c))} title={c.subject}>
                  <div class="csubject">{c.subject}</div>
                  <div class="cmeta">
                    <span class="mono">{c.short}</span> · {c.author} · {when(c.date)}
                    <span class="nums">
                      {c.files} <span class="add">+{c.added}</span> <span class="del">−{c.removed}</span>
                    </span>
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>
    </aside>
  );
}
