import { pickAndAddProject } from "./api";
import { projects, refreshProjects, refreshSessions, navStamp, selectIfStill, setError } from "./store";
import { L } from "./strings.logic";

/** Asks for a folder (native dialog, on the Rust side), adds it as a project and opens its main worktree. Shared by the sidebar and the empty states. True when a project was added. */
export const pickProject = async (): Promise<boolean> => {
  let ok = false;
  try {
    const added = await pickAndAddProject(L.pickProject);
    if (!added) return false;
    ok = true;
    const stamp = navStamp();
    await refreshProjects();
    const main = projects().find((p) => p.id === added.id)?.worktrees.find((w) => w.is_main);
    if (main) selectIfStill(stamp, main.path);
  } catch (e) {
    setError(String(e));
  }
  await refreshProjects();
  await refreshSessions();
  return ok;
};
