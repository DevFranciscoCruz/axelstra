import { invoke } from "@tauri-apps/api/core";

export type SessionInfo = { id: number; running: boolean; label: string | null };

export const call = <T = unknown>(method: string, params: Record<string, unknown> = {}) =>
  invoke<T>("daemon_call", { method, params });

export const listSessions = () => call<SessionInfo[]>("session.list");

export const spawnSession = (program: string, args: string[] = [], label?: string, cwd?: string) =>
  call<{ id: number }>("session.spawn", { program, args, label, cwd, cols: 120, rows: 30 });

export const killSession = (id: number) => call("session.kill", { id });
export const removeSession = (id: number) => call("session.remove", { id });
export const resizeSession = (id: number, cols: number, rows: number) =>
  call("session.resize", { id, cols, rows });
export const writeSession = (id: number, data: string) => invoke("session_write", { id, data });
