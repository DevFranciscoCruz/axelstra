import { invoke } from "@tauri-apps/api/core";
import { ask } from "../Confirm";
import { askRemoveProject } from "../RemoveProject";
import { doneNotice } from "../removeProjectLogic";
import { killSession, removeProject, removeSession, removeWorktree, worktreeRebase, worktreeStatus, worktreeSweep, type WtStatus } from "../api";
import { errorText } from "../appError";
import { rebaseBlocker, rebaseOutcome } from "../rebase";
import { toast } from "../toasts";
import { needsTypedName, removeRisks } from "../removeRisk";
import { P } from "../strings.panels";
import { S } from "../strings";
import {
  dismissExited,
  norm,
  refreshProjects,
  refreshSessions,
  findWorktree,
  selectWorktree,
  selectedWorktree,
  sessionsOf,
  setError,
  showNotice,
  wtStatus,
} from "../store";

const T = P.sidebar;
const F = P.flows;

/** The sidebar's actions on projects and worktrees: each one asks first when something would be lost, then refreshes the lists. */
export function createSidebarActions() {
  /** runs `f`, reports a failure, and refreshes projects and sessions either way */
  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
    } catch (e) {
      setError(String(e));
    }
    await refreshProjects();
    await refreshSessions();
  };

  const copy = async (text: string) => {
    try {
      await invoke("clipboard_write", { text });
    } catch (e) {
      setError(String(e));
    }
  };

  const closeSessions = async (path: string) => {
    for (const s of sessionsOf(path)) {
      if (s.running) await killSession(s.id);
      await removeSession(s.id);
      dismissExited(s.id);
    }
  };

  /** frees the memory of a worktree without deleting it: its terminals and agents are closed */
  const sleepWorktree = async (path: string, name: string) => {
    const live = sessionsOf(path).filter((s) => s.running);
    if (live.length > 0 && !(await ask(T.sleepAsk(name, live.length), { title: T.sleepTitle, kind: "warning" })))
      return;
    await run(() => closeSessions(path));
  };

  /** the existing sweep removes every integrated worktree, of all projects: the question says so */
  const cleanFinished = async (names: string[], reload: () => void) => {
    const ok = await ask(T.cleanAsk(names), {
      title: T.cleanTitle,
      kind: "danger",
      okLabel: T.remove,
      cancelLabel: T.cancel,
    });
    if (!ok) return;
    try {
      const done = (await worktreeSweep()).filter((x) => x.removed).length;
      showNotice(T.sweepDone(done));
    } catch (e) {
      setError(String(e));
    }
    await refreshProjects();
    await refreshSessions();
    reload();
  };

  /** asks how to remove the project (list only, or everything), then does it and says what happened */
  const dropProject = async (id: string, name: string) => {
    const mode = await askRemoveProject(id, name);
    if (!mode) return;
    // "all" closes sessions and deletes worktrees, which can take a while: say so at once
    if (mode === "all") showNotice(F.removingProject(name));
    // the daemon is about to close this project's sessions and worktrees: leave a worktree of it before it goes
    const leaving = findWorktree(selectedWorktree())?.project.id === id;
    let result: Awaited<ReturnType<typeof removeProject>> | undefined;
    let gone = true;
    await run(async () => {
      try {
        result = await removeProject(id, mode);
      } catch (e) {
        gone = false;
        throw e;
      }
      if (leaving) selectWorktree(null);
    });
    if (gone) showNotice(doneNotice(name, mode, result));
  };

  // worktrees being removed: a second click (or menu entry) while it runs is ignored
  const removing = new Set<string>();
  /** closes the sessions and removes the folder; a failure after the sessions are gone says so and offers "Tentar de novo" */
  const removeNow = async (projectId: string, path: string, name: string) => {
    removing.add(norm(path));
    showNotice(T.removing(name));
    try {
      let gone = true;
      await run(async () => {
        let closed = false;
        try {
          await closeSessions(path);
          closed = true;
          await removeWorktree(projectId, path);
        } catch (e) {
          gone = false;
          // the folder stayed (held by a process, antivirus...): the sessions are already closed, so the error says that
          if (!closed) throw e;
          toast({
            text: F.removeKept(name, errorText(e)),
            kind: "error",
            key: `remove:${path}`,
            action: { label: F.retry, run: () => void removeAgain(projectId, path, name) },
          });
          return;
        }
        if (norm(selectedWorktree()) === norm(path)) selectWorktree(null);
      });
      if (gone) showNotice(S.worktreeRemoved(name));
    } finally {
      removing.delete(norm(path));
    }
  };
  const removeAgain = async (projectId: string, path: string, name: string) => {
    if (!removing.has(norm(path))) await removeNow(projectId, path, name);
  };
  const dropWorktree = async (projectId: string, path: string, name: string) => {
    if (removing.has(norm(path))) return showNotice(T.removing(name));
    // what would be lost, so the question is not a blind "are you sure"
    let st: WtStatus | undefined = wtStatus()[norm(path)];
    // the state may not have been read yet (startup, a worktree just created): read it now; if that fails, assume the worst
    if (!st) st = await worktreeStatus([path]).then((l) => l[0], () => undefined);
    const unknown = !st;
    const live = sessionsOf(path).filter((x) => x.running).length;
    const typed = unknown || needsTypedName(st);
    const risks = unknown ? [F.statusUnknown, ...removeRisks(undefined, live)] : removeRisks(st, live);
    const ok = await ask(T.removeAsk(name, T.removeLost(risks), typed), {
      title: T.removeTitle,
      kind: "danger",
      ...(typed ? { typeToConfirm: name, okLabel: T.remove } : {}),
    });
    if (!ok || removing.has(norm(path))) return;
    await removeNow(projectId, path, name);
  };

  // worktrees being rebased: a second click while it runs is ignored
  const rebasing = new Set<string>();
  /** rebase on the default branch: the daemon refuses a busy or dirty worktree and undoes a conflicted rebase */
  const rebaseWorktree = async (projectId: string, path: string, name: string, base: string, onDone: () => void) => {
    if (rebasing.has(norm(path))) return;
    const live = sessionsOf(path).filter((x) => x.running).length;
    const blocked = rebaseBlocker(name, wtStatus()[norm(path)], live);
    if (blocked) return void toast({ text: blocked, kind: "warn" });
    if (
      !(await ask(T.rebase.ask(name, base), {
        title: T.rebase.title,
        kind: "warning",
        okLabel: T.rebase.ok,
        cancelLabel: T.cancel,
      }))
    )
      return;
    if (rebasing.has(norm(path))) return;
    rebasing.add(norm(path));
    showNotice(T.rebase.running(name, base));
    try {
      const out = rebaseOutcome(name, base, await worktreeRebase(projectId, path));
      toast({ text: out.text, kind: out.kind, sticky: out.kind === "warn" });
    } catch (e) {
      setError(T.rebase.failed(name, String(e)));
    } finally {
      rebasing.delete(norm(path));
    }
    await refreshProjects();
    await refreshSessions();
    onDone();
  };

  return { run, copy, sleepWorktree, cleanFinished, dropProject, dropWorktree, rebaseWorktree };
}

export type SidebarActions = ReturnType<typeof createSidebarActions>;
