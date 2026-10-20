import { invoke } from "@tauri-apps/api/core";

export type SessionMeta = { project?: string; worktree?: string; kind?: string };
export type SessionInfo = { id: number; running: boolean; label: string | null; meta: SessionMeta | null };
export type Worktree = { path: string; branch: string | null; head: string; is_main: boolean };
export type Project = { id: string; name: string; path: string; worktrees: Worktree[] };

export const call = <T = unknown>(method: string, params: Record<string, unknown> = {}) =>
  invoke<T>("daemon_call", { method, params });

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
