import { open } from "@tauri-apps/plugin-dialog";
import { addProject } from "./api";
import { projects, refreshProjects, refreshSessions, selectWorktree, setError } from "./store";

/** Asks for a folder, adds it as a project and opens its main worktree. Shared by the sidebar and the empty states. */
export const pickProject = async () => {
  try {
    const dir = await open({ directory: true, multiple: false, title: "Escolha a pasta do projeto" });
    if (typeof dir !== "string") return;
    const added = await addProject(dir);
    await refreshProjects();
    const main = projects().find((p) => p.id === added.id)?.worktrees.find((w) => w.is_main);
    if (main) selectWorktree(main.path);
  } catch (e) {
    setError(String(e));
  }
  await refreshProjects();
  await refreshSessions();
};
