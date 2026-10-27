import { call, defaultShell, onDaemonEvent, spawnSession, writeSession } from "./api";
import { waitShellReady } from "./shellReady";

/** The commands of a project's setup text: one per line, blank lines and `#` comments dropped. */
export function setupCommands(text: string | null | undefined): string[] {
  return (text ?? "")
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#"));
}

/** True when the setup has more than one line, which the one-line editor of the sidebar cannot show without losing the breaks. */
export const isMultiLine = (text: string | null | undefined): boolean => /\n/.test((text ?? "").trim());

/** What is typed into the shell: every command followed by Enter, so the shell runs them one after the other. */
export const setupInput = (cmds: string[]): string => cmds.map((c) => `${c}\r`).join("");

/** Opens a visible "setup" terminal in the worktree and types the commands once its prompt is up.
 *  Returns false when there was nothing to run. The output stays in the terminal for the user to read. */
export async function runSetup(projectId: string, worktree: string, text: string | null | undefined): Promise<boolean> {
  const cmds = setupCommands(text);
  if (!cmds.length) return false;
  const [program, args] = await defaultShell(worktree);
  const { id } = await spawnSession(program, args, "setup", worktree, { project: projectId, worktree, kind: "shell" });
  // type only once the shell has printed its prompt (a slow profile can take seconds); if it never does, type anyway
  void waitShellReady(() => call<{ lines: string[] }>("session.screen", { id, lines: 5 }).then((r) => r.lines), {
    changes: (fn) => onDaemonEvent("session.changed", fn),
  })
    .then(() => writeSession(id, setupInput(cmds)))
    .catch(() => 0);
  return true;
}
