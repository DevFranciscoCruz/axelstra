import { invoke } from "@tauri-apps/api/core";

export type SessionMeta = { project?: string; worktree?: string; kind?: string };
export type SessionInfo = { id: number; running: boolean; label: string | null; meta: SessionMeta | null };
export type Worktree = { path: string; branch: string | null; head: string; is_main: boolean };
export type Project = { id: string; name: string; path: string; worktrees: Worktree[];
  slug?: string;
  setup?: string | null;
  icon_url?: string | null;
  remote?: string | null;
};

export const call = <T = unknown>(method: string, params: Record<string, unknown> = {}) =>
  invoke<T>("daemon_call", { method, params });

import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
export const defaultShell = () => tauriInvoke<[string, string[]]>("default_shell");

/** Opens the chosen agent CLI straight away in a worktree. The shell stays alive after the agent exits. */
export const launchDefaultAgent = async (project: string, worktree: string, agent: string, cli: string) => {
  const [program, args] = await defaultShell();
  const run = /powershell|pwsh/i.test(program)
    ? [...args, "-NoExit", "-Command", cli]
    : [...args, "-c", `${cli}; exec "${program}" -l`];
  const { id } = await spawnSession(program, run, agent, worktree, { project, worktree, kind: "agent" });
  return id;
};

/** Change events for the selected worktree: the daemon watches the files and tells us, so git runs only when needed. */
let watchReady: Promise<void> | null = null;
const startWatch = (onChanged: (worktree: string) => void) => {
  const ch = new Channel<{ event?: string; worktree?: string }>();
  ch.onmessage = (m) => m?.event === "fs.changed" && m.worktree && onChanged(m.worktree);
  return tauriInvoke<void>("watch_start", { onEvent: ch });
};
export const watchWorktree = async (worktree: string | null, onChanged: (worktree: string) => void) => {
  watchReady ??= startWatch(onChanged);
  try {
    await watchReady;
    await tauriInvoke("watch_set", { worktree });
  } catch {
    // daemon restarted or never reachable: reconnect on the next selection
    watchReady = null;
  }
};

export const listSessions = () => call<SessionInfo[]>("session.list");

export const spawnSession = (
  program: string,
  args: string[] = [],
  label?: string,
  cwd?: string,
  meta?: SessionMeta,
) => call<{ id: number }>("session.spawn", { program, args, label, cwd, meta, cols: 120, rows: 30 });

export const killSession = (id: number) => call("session.kill", { id });
export const removeSession = (id: number) => call("session.remove", { id });
export const resizeSession = (id: number, cols: number, rows: number) =>
  call("session.resize", { id, cols, rows });
export const writeSession = (id: number, data: string) => invoke("session_write", { id, data });

export const listProjects = () => call<Project[]>("project.list");
export const addProject = (path: string) => call<Project>("project.add", { path });
export const projectLogo = (id: string) => call<{ data_url: string | null }>("project.logo", { id }).then((r) => r.data_url);
export const setProjectSetup = (id: string, setup: string) => call("project.setup", { id, setup });
export const removeProject = (id: string) => call("project.remove", { id });
export const createWorktree = (project: string, name: string) => call<Worktree>("worktree.create", { project, name });
export const removeWorktree = (project: string, path: string) => call("worktree.remove", { project, path });

export type FileChange = {
  path: string;
  status: string;
  untracked: boolean;
  staged: boolean;
  added: number;
  removed: number;
  binary: boolean;
};
export type Changes = { against: string; files: FileChange[] };

export const gitChanges = (worktree: string, base?: string) => call<Changes>("git.changes", { worktree, base });
export const gitDiff = (worktree: string, path: string, base?: string) =>
  call<{ diff: string }>("git.diff", { worktree, path, base }).then((r) => r.diff);

export type FsEntry = { name: string; dir: boolean };
export type Opened = { text: string; eol: "lf" | "crlf"; bom: boolean; hash: string };
export type Saved = { hash?: string; conflict?: boolean; disk_hash?: string };

export const fsList = (worktree: string, path: string) =>
  call<{ entries: FsEntry[] }>("fs.list", { worktree, path }).then((r) => r.entries);
export const fsRead = (worktree: string, path: string) => call<Opened>("fs.read", { worktree, path });
export const fsStat = (worktree: string, path: string) =>
  call<{ hash: string | null }>("fs.stat", { worktree, path }).then((r) => r.hash);
export const fsWrite = (worktree: string, path: string, text: string, eol: string, bom: boolean, base_hash?: string) =>
  call<Saved>("fs.write", { worktree, path, text, eol, bom, base_hash });

export const gitCommit = (worktree: string, message: string) => call<{ sha: string }>("git.commit", { worktree, message });
export const gitPush = (worktree: string) => call<{ output: string }>("git.push", { worktree });
export const gitPr = (worktree: string, title: string, body: string, base?: string) =>
  call<{ url: string }>("git.pr", { worktree, title, body, base }).then((r) => r.url);

export type KCard = {
  meta: { id: string; title: string; status: string; agent: string; depends_on: string[]; priority: number };
  body: string;
};
export type Board = { cards: KCard[]; broken: { file: string; error: string }[] };

export const kanbanList = (project: string) => call<Board>("kanban.list", { project });
export const kanbanSave = (project: string, card: Record<string, unknown>) => call<KCard>("kanban.save", { project, ...card });
export const kanbanMove = (project: string, id: string, status: string) => call("kanban.move", { project, id, status });
export const kanbanDelete = (project: string, id: string) => call("kanban.delete", { project, id });

export type OrchRun = {
  state: "idle" | "running" | "done" | "error";
  name?: string;
  integ_branch?: string;
  integ_wt?: string;
  merged?: string[];
  failed?: [string, string][];
  needs_review?: [string, string][];
  blocked?: string[];
  error?: string;
};
export const orchStart = (project: string, max = 3) => call<OrchRun>("orch.start", { project, max });
export const orchStatus = (project: string) => call<OrchRun>("orch.status", { project });

export type Commit = {
  sha: string;
  short: string;
  author: string;
  date: string;
  subject: string;
  files: number;
  added: number;
  removed: number;
};
export const gitLog = (worktree: string, base?: string) =>
  call<{ commits: Commit[] }>("git.log", { worktree, base }).then((r) => r.commits);
export const gitShow = (worktree: string, sha: string) =>
  call<{ diff: string }>("git.show", { worktree, sha }).then((r) => r.diff);
