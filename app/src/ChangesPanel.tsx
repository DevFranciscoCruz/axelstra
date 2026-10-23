import { K } from "./keyboard";
import { Button, Input, Textarea } from "./ui";
import { RightTabs } from "./RightTabs";
import { ShipPanel } from "./ShipPanel";
import { Icon } from "./Icon";
import { ask } from "./Confirm";
import { For, Show, createResource, createSignal } from "solid-js";
import { Commit, FileChange, gitCommit, gitLog, gitPr, gitPush, gitSuggest } from "./api";
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
  rightTab,
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
  const [suggesting, setSuggesting] = createSignal(false);
  const [note, setNote] = createSignal<{ ok: boolean; text: string } | null>(null);
  const [showChanges, setShowChanges] = createSignal(true);
  const [showCommits, setShowCommits] = createSignal(true);

  const [log, { refetch: refetchLog }] = createResource(
    () => ({ wt: selectedWorktree(), base: commitsBase(), n: changes()?.files.length ?? 0 }),
    (k) => (k.wt ? gitLog(k.wt, k.base) : []),
  );

  // asks the agent for a message that fits the changes; the user reads and edits it before committing
  const suggest = async () => {
    const wt = selectedWorktree();
    if (!wt || suggesting()) return;
    setSuggesting(true);
    setNote(null);
    try {
      const m = await gitSuggest(wt);
      if (selectedWorktree() === wt) setMessage(m);
    } catch (e) {
      setNote({ ok: false, text: String(e) });
    } finally {
      setSuggesting(false);
    }
  };

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
    <aside class="rightbar" id="rpanel" role="tabpanel" aria-labelledby={`rtab-${rightTab()}`}>
      <RightTabs />

      <div class="sc-head">
        <Button variant="secondary" size={null} class="pill rounded-full px-3 py-1 border-border" disabled={busy()} onClick={() => setPrOpen(!prOpen())}>
          <Icon name="branch" size={13} /> Criar PR
        </Button>
        <span class="grow" />
        <Button variant="ghost" size="icon"
          class="icon"
          title="Atualizar" aria-label="Atualizar"
          onClick={() => {
            void refreshChanges();
            void refetchLog();
          }}
        >
          <Icon name="refresh" />
        </Button>
        <Button variant="ghost" size="icon"
          class="icon"
          title="Mais ações" aria-label="Mais ações"
          onClick={(e) =>
            showMenu(e, [
              {
                label: "Por consolidar (vs HEAD)",
                icon: changesMode() === "head" ? "check" : "",
                run: () => {
                  setChangesMode("head");
                  void refreshChanges();
                },
              },
              {
                label: "Todo o branch (vs principal)",
                icon: changesMode() === "main" ? "check" : "",
                run: () => {
                  setChangesMode("main");
                  void refreshChanges();
                },
              },
              { separator: true },
              { label: "Push", icon: "arrowUp", run: () => doPush() },
            ])
          }
        >
          <Icon name="more" />
        </Button>
      </div>
      <div class="sc-branch" title={branch()}>
        <div class="sc-name">{branch() || "(destacado)"}</div>
        <div class="sc-base"><Icon name="arrowRight" size={11} /> {base()}</div>
      </div>

      <Show when={prOpen()}>
        <div class="prow sc-pr">
          <Input
            placeholder="Título do pull request"
            value={prTitle()}
            onInput={(e) => setPrTitle(e.currentTarget.value)}
            onKeyDown={(e) => e.key === "Enter" && prTitle().trim() && void doPr()}
            ref={(el) => queueMicrotask(() => el.focus())}
          />
          <Button size="sm" disabled={busy() || !prTitle().trim()} onClick={() => void doPr()}>
            Criar
          </Button>
        </div>
      </Show>

      <div class="sc-commit">
        <div class="sc-msg">
          <Textarea
            placeholder={suggesting() ? "A gerar mensagem…" : K("Mensagem (Ctrl+Enter para fazer commit)")}
            rows={3}
            value={message()}
            onInput={(e) => setMessage(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && canCommit()) void doCommit();
            }}
          />
          <Button variant="ghost" size="icon"
            class="icon sc-ai"
            classList={{ busy: suggesting() }}
            disabled={suggesting() || busy() || files().length === 0}
            title={files().length === 0 ? "Sem alterações para descrever" : "Sugerir mensagem com IA (usa o claude)"}
            aria-label="Sugerir mensagem com IA"
            onClick={() => void suggest()}
          >
            <Icon name="sparkle" size={14} />
          </Button>
        </div>
        <div class="sc-split">
          <Button variant="secondary" size={null} class="sc-main flex-1 p-2 rounded-l-lg rounded-r-none border-border" disabled={!canCommit()} title={canCommit() ? K("Commit (Ctrl+Enter)") : files().length === 0 ? "Sem alterações para commit" : message().trim() ? "A trabalhar…" : "Escreve uma mensagem"} onClick={() => void doCommit()}>
            <Icon name="check" /> Commit
          </Button>
          <Button
            variant="secondary"
            size={null}
            class="sc-drop w-8 p-0 rounded-r-lg rounded-l-none border-l-0 border-border"
            disabled={busy()}
            title="Mais opções de commit" aria-label="Mais opções de commit"
            onClick={(e) =>
              showMenu(e, [
                { label: "Commit", icon: "check", disabled: !canCommit(), run: () => doCommit() },
                { label: "Commit e Push", icon: "arrowUp", disabled: !canCommit(), run: () => doCommitPush() },
                { label: "Push", icon: "arrowUp", run: () => doPush() },
              ])
            }
          >
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M2 3.5 5 6.5 8 3.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
          </Button>
        </div>
        <Show when={note()}>
          {(n) => (
            <div class="pnote" role="status" aria-live="polite" classList={{ bad: !n().ok }} title={n().text}>
              {n().text}
            </div>
          )}
        </Show>
      </div>

      <ShipPanel />

      <div class="sc-body">
        <div class="sc-title" {...press(() => setShowChanges(!showChanges()))} aria-expanded={showChanges()}>
          <span class="chev"><Icon name={showChanges() ? "chevDown" : "chevRight"} /></span>
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
                    <span class="ficon"><Icon name="file" size={12} /></span>
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
          <span class="chev"><Icon name={showCommits() ? "chevDown" : "chevRight"} /></span>
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
