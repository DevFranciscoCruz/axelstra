import { invoke } from "@tauri-apps/api/core";
import { ask } from "../Confirm";
import { killSession, removeProject, removeSession, removeWorktree, worktreeSweep } from "../api";
import { needsTypedName, removeRisks } from "../removeRisk";
import { P } from "../strings.panels";
import { S } from "../strings";
import {
  norm,
  refreshProjects,
  refreshSessions,
  selectWorktree,
  selectedWorktree,
  sessionsOf,
  setError,
  showNotice,
  wtStatus,
} from "../store";

const T = P.sidebar;

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

  const dropProject = async (id: string, name: string) => {
    if (
      !(await ask(T.dropAsk(name), { title: T.dropTitle, kind: "warning", okLabel: T.dropOk, cancelLabel: T.cancel }))
    )
      return;
    let gone = true;
    await run(async () => {
      try {
        await removeProject(id);
      } catch (e) {
        gone = false;
        throw e;
      }
    });
    if (gone) showNotice(S.projectRemoved(name));
  };

  // worktrees being removed: a second click (or menu entry) while it runs is ignored
  const removing = new Set<string>();
  const dropWorktree = async (projectId: string, path: string, name: string) => {
    if (removing.has(norm(path))) return showNotice(T.removing(name));
    // what would be lost, so the question is not a blind "are you sure"
    const st = wtStatus()[norm(path)];
    const live = sessionsOf(path).filter((x) => x.running).length;
    const typed = needsTypedName(st);
    const ok = await ask(T.removeAsk(name, T.removeLost(removeRisks(st, live)), typed), {
      title: T.removeTitle,
      kind: "danger",
      ...(typed ? { typeToConfirm: name, okLabel: T.remove } : {}),
    });
    if (!ok || removing.has(norm(path))) return;
    removing.add(norm(path));
    showNotice(T.removing(name));
    try {
      let gone = true;
      await run(async () => {
        try {
          await closeSessions(path);
          await removeWorktree(projectId, path);
        } catch (e) {
          gone = false;
          throw e;
        }
        if (norm(selectedWorktree()) === norm(path)) selectWorktree(null);
      });
      if (gone) showNotice(S.worktreeRemoved(name));
    } finally {
      removing.delete(norm(path));
    }
  };

  return { run, copy, sleepWorktree, cleanFinished, dropProject, dropWorktree };
}

export type SidebarActions = ReturnType<typeof createSidebarActions>;
