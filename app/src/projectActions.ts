import { pickAndAddProject } from "./api";
import { projects, refreshProjects, refreshSessions, selectWorktree, setError } from "./store";
import { L } from "./strings.logic";

/** Asks for a folder (native dialog, on the Rust side), adds it as a project and opens its main worktree. Shared by the sidebar and the empty states. */
export const pickProject = async () => {
  try {
    const added = await pickAndAddProject(L.pickProject);
    if (!added) return;
    await refreshProjects();
    const main = projects().find((p) => p.id === added.id)?.worktrees.find((w) => w.is_main);
    if (main) selectWorktree(main.path);
  } catch (e) {
    setError(String(e));
  }
  await refreshProjects();
  await refreshSessions();
};
