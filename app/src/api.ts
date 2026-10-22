import { invoke } from "@tauri-apps/api/core";

export type SessionMeta = { project?: string; worktree?: string; kind?: string; agent?: string; resume?: string; key?: string; parent?: number; worker?: boolean };
export type SessionInfo = { id: number; running: boolean; idle_ms?: number; waiting?: boolean; escalated?: boolean; unread?: number; finished?: boolean; label: string | null; meta: SessionMeta | null };
export type Worktree = { path: string; branch: string | null; head: string; is_main: boolean };
export type Project = { id: string; name: string; path: string; worktrees: Worktree[];
  /** false for a plain folder that is not a git repository (yet) */
  git?: boolean;
  slug?: string;
  setup?: string | null;
  default_branch?: string;
  icon_url?: string | null;
  remote?: string | null;
};

export const call = <T = unknown>(method: string, params: Record<string, unknown> = {}) =>
  invoke<T>("daemon_call", { method, params });

/** same, on a second connection: for slow polling that must not hold up the interactive calls */
export const callBg = <T = unknown>(method: string, params: Record<string, unknown> = {}) =>
  invoke<T>("daemon_call_bg", { method, params });

import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
export const defaultShell = () => tauriInvoke<[string, string[]]>("default_shell");

/** Command line for an agent. On restore it resumes the earlier conversation, falling back to a fresh start. */
const agentLine = (cli: string, resume: string | undefined, restoring: boolean, ps: boolean, hooks: string) => {
  const claude = (flags: string) => `claude --settings '${hooks}' ${flags}`;
  const fresh = cli === "claude" && resume ? claude(`--session-id ${resume}`) : cli;
  if (!restoring) return fresh;
  const back = cli === "claude" && resume ? claude(`--resume ${resume}`) : cli === "codex" ? "codex resume --last" : cli;
  return ps ? `${back}; if (!$?) { ${fresh} }` : `${back} || ${fresh}`;
};

/** Opens the chosen agent CLI straight away in a worktree. The shell stays alive after the agent exits. */
export const launchDefaultAgent = async (project: string, worktree: string, agent: string, cli: string, restore?: { key?: string; resume?: string }) => {
  const [program, args] = await defaultShell();
  const ps = /powershell|pwsh/i.test(program);
  // key is stable for the life of the tab; resume is the conversation id, which /clear replaces
  const key = restore ? restore.key : cli === "claude" ? crypto.randomUUID() : undefined;
  const resume = restore ? restore.resume ?? key : key;
  const { dir } = await call<{ dir: string }>("app.dir");
  const line = agentLine(cli, resume, !!restore, ps, `${dir}/hooks.json`);
  const run = ps ? [...args, "-NoExit", "-Command", line] : [...args, "-c", `${line}; exec "${program}" -l`];
  const env = key ? { AXELSTRA_KEY: key, AXELSTRA_DIR: dir } : undefined;
  const { id } = await spawnSession(program, run, agent, worktree, { project, worktree, kind: "agent", agent: cli, resume, key }, env);
  return id;
};

/** Types a note into a resumed agent once its screen has settled after booting. Fire and forget. */
const nudge = async (id: number, text: string) => {
  const start = Date.now();
  await new Promise((r) => setTimeout(r, 4000));
  while (Date.now() - start < 90000) {
    const s = (await listSessions()).find((x) => x.id === id);
    if (!s || !s.running) return;
    if ((s.idle_ms ?? 0) >= 2500) break;
    await new Promise((r) => setTimeout(r, 1000));
  }
  await writeSession(id, text);
  await new Promise((r) => setTimeout(r, 400));
  await writeSession(id, "\r");
};

/** What an agent should be told about the restart: it was mid-task, and/or workers it started were stopped. */
export const restartNote = (interrupted: boolean, lost: string[]) => {
  const parts: string[] = [];
  if (interrupted) parts.push("Axelstra was restarted and this conversation was resumed. You were in the middle of a task: continue where you left off.");
  if (lost.length) {
    parts.push(
      `Workers you started were stopped by the restart and are gone: ${lost.join(", ")}. Their worktrees keep what they had already done. Run \`axelstra ls\` and relaunch what is unfinished (a running \`axelstra flow\` stopped too: run it again with \`--resume\` and it skips what finished).`,
    );
  }
  return parts.length ? `[axelstra] ${parts.join(" ")}` : "";
};

/** Brings back the sessions of a previous daemon run (reboot, installer): shells reopen, agents resume their conversation. */
export const restoreSessions = async (): Promise<{ restored: number; skipped: number; failed: string[] }> => {
  const pending = await call<{ program: string; args?: string[]; cwd?: string; label?: string; meta?: SessionMeta }[]>("session.pending");
  let restored = 0;
  let skipped = 0;
  const failed: string[] = [];
  for (const r of pending) {
    const m = r.meta;
    try {
      if (m?.kind === "agent" && m.agent && m.project && m.worktree) {
        // a cleared / never used conversation has nothing to bring back: no tab, no process
        if ((m as { blank?: boolean }).blank) {
          skipped++;
        } else {
          const id = await launchDefaultAgent(m.project, m.worktree, r.label ?? m.agent, m.agent, { key: m.key, resume: m.resume });
          const x = m as { interrupted?: boolean; lost_workers?: string[] };
          const note = restartNote(!!x.interrupted, x.lost_workers ?? []);
          if (note) void nudge(id, note).catch(() => 0);
          restored++;
        }
      } else {
        await spawnSession(r.program, r.args ?? [], r.label ?? undefined, r.cwd, m ?? undefined, (r as { env?: Record<string, string> }).env);
        restored++;
      }
    } catch {
      // usually a worktree that no longer exists
      failed.push(r.label ?? m?.worktree ?? r.cwd ?? r.program);
    }
    // this entry is dealt with either way; the others stay listed until their turn, so a crash midway loses nothing
    await call("session.pending_remove", { rec: r }).catch(() => 0);
  }
  return { restored, skipped, failed };
};

/** Change events for the selected worktree: the daemon watches the files and tells us, so git runs only when needed. */
let watchReady: Promise<void> | null = null;
const startWatch = (onChanged: (worktree: string) => void, onSessions?: () => void) => {
  const ch = new Channel<{ event?: string; worktree?: string }>();
  ch.onmessage = (m) => {
    if (m?.event === "fs.changed" && m.worktree) onChanged(m.worktree);
    else if (m?.event === "session.changed") onSessions?.();
  };
  return tauriInvoke<void>("watch_start", { onEvent: ch });
};
export const watchWorktree = async (worktree: string | null, onChanged: (worktree: string) => void, onSessions?: () => void) => {
  watchReady ??= startWatch(onChanged, onSessions);
  try {
    await watchReady;
    await tauriInvoke("watch_set", { worktree });
  } catch {
    // daemon restarted or never reachable: reconnect on the next selection
    watchReady = null;
  }
};

export const listSessions = () => call<SessionInfo[]>("session.list");

/** Size of the last terminal that was on screen. A new session starts at this size, so the agent draws its first screen
 * right and is not resized a moment later (a resize leaves part of the old screen behind). */
let lastTerm = { cols: 0, rows: 0 };
export const rememberTermSize = (cols: number, rows: number) => {
  if (cols >= 20 && rows >= 5) lastTerm = { cols, rows };
};
const startSize = () =>
  lastTerm.cols
    ? lastTerm
    : // nothing on screen yet (restoring at startup): the terminal area is roughly the window minus the side panels
      { cols: Math.max(80, Math.floor((window.innerWidth * 0.7 - 16) / 7.8)), rows: Math.max(24, Math.floor((window.innerHeight - 110) / 15)) };

export const spawnSession = (
  program: string,
  args: string[] = [],
  label?: string,
  cwd?: string,
  meta?: SessionMeta,
  env?: Record<string, string>,
) => call<{ id: number }>("session.spawn", { program, args, label, cwd, meta, env, ...startSize() });

export const killSession = (id: number) => call("session.kill", { id });
export const removeSession = (id: number) => call("session.remove", { id });
export const resizeSession = (id: number, cols: number, rows: number) =>
  call("session.resize", { id, cols, rows });
export const writeSession = (id: number, data: string) => invoke("session_write", { id, data });

export const listProjects = () => call<Project[]>("project.list");
export const addProject = (path: string) => call<Project>("project.add", { path });
export const projectLogo = (id: string) => call<{ data_url: string | null }>("project.logo", { id }).then((r) => r.data_url);
export const setProjectSetup = (id: string, setup: string) => call("project.setup", { id, setup });
export const detectPackageManagers = (id: string) => call<{ installed: string[]; suggested: string | null }>("pm.detect", { id });
export const removeProject = (id: string) => call("project.remove", { id });
export const createWorktree = (project: string, name: string) => call<Worktree>("worktree.create", { project, name });
export type GhItem = { number: number; title: string; author: string; url: string; state: string; draft: boolean; branch: string; checks: string; review: string };
export const ghList = (project: string, kind: "pr" | "issue") => call<{ items: GhItem[] }>("gh.list", { project, kind }).then((r) => r.items);
export const ghListBg = (project: string, kind: "pr" | "issue") => callBg<{ items: GhItem[] }>("gh.list", { project, kind }).then((r) => r.items);
export const worktreeFromPr = (project: string, number: number) => call<Worktree>("worktree.from_pr", { project, number });
export const listPorts = (worktree: string) => call<{ ports: { port: number; pid: number }[] }>("ports.list", { worktree }).then((r) => r.ports);
export type Schedule =
  | { kind: "interval"; minutes: number }
  | { kind: "daily"; time: string }
  | { kind: "weekly"; day: number; time: string };
export type Automation = { id: string; name: string; project: string; agent: string; prompt: string; schedule: Schedule; enabled: boolean; last_run: number };
export const listAutomations = () => call<{ automations: Automation[] }>("automation.list").then((r) => r.automations);
export const saveAutomation = (a: Automation) => call<Automation>("automation.save", a as unknown as Record<string, unknown>);
export const deleteAutomation = (id: string) => call("automation.delete", { id });
export const runAutomation = (id: string) => call<{ session: number; worktree: string }>("automation.run", { id });
export const openInEditor = (worktree: string, app: "code") => call("open.editor", { worktree, app });
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
export const fsSearch = (worktree: string, query: string) =>
  call<{ files: string[] }>("fs.search", { worktree, query }).then((r) => r.files);
export type GrepHit = { path: string; line: number; text: string };
export const fsGrep = (worktree: string, query: string) => call<{ matches: GrepHit[] }>("fs.grep", { worktree, query }).then((r) => r.matches);
export const fsCreate = (worktree: string, path: string, dir: boolean) => call("fs.create", { worktree, path, dir });
export const fsRename = (worktree: string, path: string, to: string) => call("fs.rename", { worktree, path, to });
export const fsDelete = (worktree: string, path: string) => call("fs.delete", { worktree, path });
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

export type WtStatus = { path: string; dirty: number; ahead: number; behind: number; upstream: boolean };
export const worktreeStatus = (paths: string[]) => callBg<{ status: WtStatus[] }>("worktree.status", { paths }).then((r) => r.status);

export type AgentHit = { agent: "claude" | "codex"; session: string; cwd: string; ts: number; snippet: string };
export const agentSearch = (query: string, max = 30) => callBg<{ hits: AgentHit[] }>("agent.search", { query, max }).then((r) => r.hits);

export type TokenUsage = { input: number; output: number; cache_read: number; cache_write: number; messages: number; context: number };
export const agentUsage = (cwd: string, session?: string) => callBg<{ usage: TokenUsage | null }>("agent.usage", { cwd, session }).then((r) => r.usage);

export const computerStatus = () => call<{ enabled: boolean }>("computer.status").then((r) => r.enabled);
export const computerEnable = (on: boolean) => call<{ enabled: boolean }>("computer.enable", { on }).then((r) => r.enabled);

export type WorkerLimits = { max_workers: number; workers: number };
export type SessionRes = { id: number; mem: number; cpu_100ns: number };
export const sessionResources = () => callBg<{ sessions: SessionRes[]; cores: number }>("session.resources");

/** tasks of running `axelstra flow` plans: queued (waiting for its turn), running, done, failed, blocked */
export type FlowTask = { id: string; worktree: string; state: string };
export const flowTasks = () => callBg<{ tasks: FlowTask[] }>("flow.list").then((r) => r.tasks);

export const workerLimits = (max?: number) => call<WorkerLimits>("orch.limits", max === undefined ? {} : { max_workers: max });

/** What Claude Code's status line last reported for an agent tab (context window, 5 h and weekly limits). */
export type ClaudeStatus = {
  context_window?: { context_window_size?: number; used_percentage?: number };
  rate_limits?: { five_hour?: { used_percentage?: number; resets_at?: number }; seven_day?: { used_percentage?: number; resets_at?: number } };
};
export const agentStatus = (key: string) => callBg<{ status: ClaudeStatus | null }>("agent.status", { key }).then((r) => r.status);

export type CodexWindow = { used_percent?: number; window_minutes?: number; resets_at?: number; resets_in_seconds?: number };
export type CodexLimits = { primary?: CodexWindow; secondary?: CodexWindow; as_of?: number };
export const codexLimits = () => callBg<{ limits: CodexLimits | null }>("codex.limits").then((r) => r.limits);
