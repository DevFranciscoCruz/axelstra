import type { Timeline } from "./timelineLogic";
import type { OverlapFiles, RebaseResult } from "./rebase";
import type { WorkspaceConflicts, WorkspaceMode, WorkspaceOutcome, WorkspacePreview } from "./workspaceLogic";
import type { WtUsage } from "./usageFormat";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AppError, toAppError } from "./appError";
import { apiContext } from "./apiContext";
import { hostOf, norm } from "./paths";
import { coded } from "./msg";

/** Mirrors the Rust `meta` of a session (crates/daemon). `blank`, `interrupted` and `lost_workers` are set on entries of `session.pending`. */
export type SessionMeta = {
  project?: string;
  worktree?: string;
  kind?: string;
  agent?: string;
  resume?: string;
  key?: string;
  note?: string;
  parent?: number;
  worker?: boolean;
  main?: boolean;
  /** first agent of a worktree: named "main", parent of its tasks */ lead?: boolean;
  /** a cleared / never used conversation: nothing to bring back */
  blank?: boolean;
  /** the agent was mid-task when the daemon stopped */
  interrupted?: boolean;
  /** workers this agent started that the restart stopped */
  lost_workers?: string[];
};
/** Set by the daemon on an agent that went quiet without reporting: the notice in message-code form (see msg/agents.ts). */
export type StallInfo = { minutes: number; code: string; params: Record<string, unknown>; text: string };
export type SessionInfo = {
  id: number;
  running: boolean;
  idle_ms?: number;
  waiting?: boolean;
  stalled?: boolean;
  stall?: StallInfo | null;
  escalated?: boolean;
  unread?: number;
  finished?: boolean;
  label: string | null;
  meta: SessionMeta | null;
};
export type Worktree = {
  path: string;
  branch: string | null;
  head: string;
  is_main: boolean;
  /** worktree of the same project this one is grouped under */ parent?: string | null;
};
export type Project = {
  id: string;
  name: string;
  path: string;
  worktrees: Worktree[];
  /** false for a plain folder that is not a git repository (yet) */
  git?: boolean;
  slug?: string;
  setup?: string | null;
  default_branch?: string;
  icon_url?: string | null;
  remote?: string | null;
  /** id of the remote host the project lives on; absent = this machine */
  host?: string;
  /** absent = on: finished ax/* work is integrated into the default branch by itself */
  auto_integrate?: boolean;
  /** absent = off: never pushes unless set */
  push_after_integrate?: boolean;
};

/** A call that gets no answer in this long fails (a stuck daemon must not leave the UI waiting for ever). */
export const CALL_TIMEOUT_MS = 30_000;
/** Methods that legitimately run git, gh, docker or a whole agent run: they get longer. */
export const SLOW_TIMEOUT_MS = 5 * 60_000;
const SLOW =
  /^(git\.|gh\.|docker\.|pm\.|orch\.|worktree\.(create|from_pr|remove|sweep|rename|integrate)|project\.(add|remove)|agent\.|fs\.grep|automation\.run|diagnostics)/;
export const timeoutFor = (method: string) => (SLOW.test(method) ? SLOW_TIMEOUT_MS : CALL_TIMEOUT_MS);

/** `p`, but failing with a readable AppError when it rejects or takes longer than the method's timeout. */
const guard = <T>(method: string, p: Promise<T>): Promise<T> => {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const late = new Promise<never>((_, rej) => {
    timer = setTimeout(
      () =>
        rej(
          new AppError(
            method,
            `Sem resposta do daemon (${method}) ao fim de ${Math.round(timeoutFor(method) / 1000)} s.`,
          ),
        ),
      timeoutFor(method),
    );
  });
  return Promise.race([p, late])
    .catch((e) => Promise.reject(toAppError(method, e)))
    .finally(() => clearTimeout(timer));
};

export const call = <T = unknown>(method: string, params: Record<string, unknown> = {}) =>
  guard(method, invoke<T>("daemon_call", { method, params }));

/** same, on a second connection: for slow polling that must not hold up the interactive calls */
export const callBg = <T = unknown>(method: string, params: Record<string, unknown> = {}) =>
  guard(method, invoke<T>("daemon_call_bg", { method, params }));

/** true when the running daemon is older than the app and lacks `method` */
export const isUnknownMethod = (e: unknown) => /unknown method/i.test(e instanceof Error ? e.message : String(e));
/** `call` that answers `fallback` instead of failing when the daemon does not know the method yet */
export const callOr = <T>(method: string, params: Record<string, unknown>, fallback: T): Promise<T> =>
  call<T>(method, params).catch((e) => (isUnknownMethod(e) ? fallback : Promise.reject(e)));

import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { L } from "./strings.logic";
export type Terse = "off" | "short" | "tiny";
export type NotifyMode = "all" | "waiting" | "off";
export type ShellChoice = "auto" | "pwsh" | "powershell" | "bash" | "wsl" | "cmd";
export type Settings = {
  terse: Terse;
  rules: string | null;
  pm: string | null;
  installed: string[];
  auto_install: boolean;
  /** absent on a daemon from before these keys existed */
  notify_sound?: boolean;
  notify_mode?: NotifyMode;
  shell?: ShellChoice;
  stall_minutes?: number;
  /** UI choices the webview keeps (pins, folded groups, notifications...): `axelstra.*` key -> saved string */
  ui?: Record<string, string>;
  /** token limits per day / week; absent on a daemon from before budgets */
  budgets?: Budgets;
};
export type SettingsPatch = {
  terse?: Terse;
  pm?: string | null;
  auto_install?: boolean;
  notify_sound?: boolean;
  notify_mode?: NotifyMode;
  shell?: ShellChoice;
  stall_minutes?: number;
  budgets?: Budgets;
};
/** `terse` asks for the rules of that level (a project's own choice) instead of the saved one */
export const getSettings = (terse?: Terse) => call<Settings>("settings.get", terse ? { terse } : {});
/** `ui`: UI state to store (`null` removes a key), sent by uiState.ts only, so it is not part of what the Settings screen merges */
export const setSettings = (patch: SettingsPatch & { ui?: Record<string, string | null> }) =>
  call("settings.set", patch);

/** Result of `fn`, remembered once it succeeded; a failure is not remembered, so the next call tries again. */
const once = <T>(fn: () => Promise<T>) => {
  let p: Promise<T> | null = null;
  const get = () => (p ??= fn().catch((e) => ((p = null), Promise.reject(e))));
  get.reset = () => {
    p = null;
  };
  return get;
};
/** the login shell, one answer per host ("" = this machine) */
const shells = new Map<string, ReturnType<typeof once<[string, string[]]>>>();
const dirOnce = once(() => call<{ dir: string }>("app.dir").then((r) => r.dir));
/** The shell for a terminal in `path`: a remote path (`@host:/dir`) gets that host's login shell, anything else the local one. */
export const defaultShell = (path?: string | null): Promise<[string, string[]]> => {
  const host = hostOf(path) ?? "";
  let get = shells.get(host);
  if (!get) {
    get = once(() => tauriInvoke<[string, string[]]>("default_shell", host ? { host } : {}));
    shells.set(host, get);
  }
  return get();
};
/** the shell choice in Settings changed: the next terminal asks again */
export const resetShellCache = () => shells.clear();
export const daemonShutdown = () => call("daemon.shutdown");

/** What the Rust `diagnostics` command reports. */
export type Diagnostics = {
  app_version: string;
  app_proto: number;
  daemon_connected: boolean;
  daemon_proto: number | null;
  daemon_instance: number | null;
  daemon_build: string | null;
  daemon_security: number | null;
  app_security_min: number;
  data_dir: string;
  lag_log: string;
  lag_log_bytes: number;
  missing_tools: string[];
};
export const diagnostics = () => tauriInvoke<Diagnostics>("diagnostics");
/** Sends a webview error to the Rust log file. */
export const logFrontendError = (r: { kind: string; message: string; stack: string }) =>
  tauriInvoke<void>("log_frontend_error", r);
/** Text bundle for bug reports (version, OS, data dir, log tails). */
export const diagnosticsBundle = () => tauriInvoke<string>("diagnostics_bundle");
/** Copies the diagnostics bundle to the clipboard and returns it. */
export async function copyDiagnostics(): Promise<string> {
  const text = await diagnosticsBundle();
  await tauriInvoke("clipboard_write", { text });
  return text;
}

/** Single-quotes `s` for the shell that will read the line: PowerShell doubles a quote (the typographic ones count too),
 * sh closes the string, escapes the quote and opens it again. */
export const shQuote = (s: string, ps: boolean) =>
  ps ? `'${s.replace(/['‘’‚‛]/g, (q) => q + q)}'` : `'${s.replace(/'/g, "'\\''")}'`;

/** How each non-claude agent continues its latest conversation (mirrors the resume column of crates/core/src/agents.rs). */
const RESUME_LINE: Record<string, string> = {
  codex: "codex resume --last",
  gemini: "gemini --resume latest",
};

/** A search hit resumes a specific Gemini conversation as "gemini:SESSION_ID" (any other resume value is Axelstra's own tab key). */
export const geminiResumeId = (resume: string | undefined) => {
  const id = resume?.startsWith("gemini:") ? resume.slice(7) : "";
  return /^[A-Za-z0-9_-]{1,80}$/.test(id) ? id : null;
};

/** Command line for an agent. On restore it resumes the earlier conversation, falling back to a fresh start. */
export const agentLine = (
  cli: string,
  resume: string | undefined,
  restoring: boolean,
  ps: boolean,
  hooks: string,
  rules?: string | null,
) => {
  const style = rules ? ` --append-system-prompt ${shQuote(rules, ps)}` : "";
  const claude = (flags: string) => `claude --settings ${shQuote(hooks, ps)}${style} ${flags}`;
  const fresh = cli === "claude" && resume ? claude(`--session-id ${resume}`) : cli;
  if (!restoring) return fresh;
  const gem = cli === "gemini" ? geminiResumeId(resume) : null;
  const back =
    cli === "claude" && resume
      ? claude(`--resume ${resume}`)
      : gem
        ? `gemini --resume ${gem}`
        : (RESUME_LINE[cli] ?? cli);
  return ps ? `${back}; if (!$?) { ${fresh} }` : `${back} || ${fresh}`;
};

/** Opens the chosen agent CLI straight away in a worktree. The shell stays alive after the agent exits.
 * `pre.rules` lets a batch (restore) read the settings once instead of once per session.
 * `terse` is the project's own reply style; undefined follows the global setting. */
export const launchDefaultAgent = async (
  project: string,
  worktree: string,
  agent: string,
  cli: string,
  restore?: { key?: string; resume?: string; note?: string; lead?: boolean },
  pre?: { rules: string | null },
  terse?: Terse,
) => {
  const [[program, args], dir, rules] = await Promise.all([
    defaultShell(worktree),
    dirOnce(),
    pre
      ? pre.rules
      : getSettings(terse).then(
          (x) => x.rules,
          () => null,
        ),
  ]);
  const ps = /powershell|pwsh/i.test(program);
  // the main worktree is not a task: its agent gets no "Tarefa N" name and no task file
  const main =
    apiContext
      .projects()
      .find((p) => p.id === project)
      ?.worktrees.some((w) => w.is_main && norm(w.path) === norm(worktree)) || undefined;
  // key is stable for the life of the tab; resume is the conversation id, which /clear replaces
  const key = restore ? restore.key : cli === "claude" ? crypto.randomUUID() : undefined;
  const resume = restore ? (restore.resume ?? key) : key;
  const line = agentLine(cli, resume, !!restore, ps, `${dir}/hooks.json`, rules);
  const run = ps
    ? [...args, "-NoExit", "-Command", line]
    : [...args, "-c", `${line}; exec ${shQuote(program, false)} -l`];
  const env = key ? { AXELSTRA_KEY: key, AXELSTRA_DIR: dir } : undefined;
  const { id } = await spawnSession(
    program,
    run,
    agent,
    worktree,
    {
      project,
      worktree,
      kind: "agent",
      agent: cli,
      resume,
      key,
      note: restore?.note,
      main,
      lead: restore?.lead || undefined,
    },
    env,
  );
  // only claude takes the rules as a system prompt: type them into the others once they have booted
  if (rules && cli !== "claude" && !restore) void nudge(id, rules).catch((e) => console.debug("nudge", e));
  return id;
};

/** One session.list on the second connection shared by everyone who asks within a second: restoring N agents polls once, not N times. */
let sharedList: { at: number; p: Promise<SessionInfo[]> } | null = null;
const sessionsShared = () => {
  const now = Date.now();
  if (!sharedList || now - sharedList.at > 900) sharedList = { at: now, p: callBg<SessionInfo[]>("session.list") };
  return sharedList.p;
};

/** nudges waiting or about to type, by session: closing the session or typing in it cancels the nudge */
const nudges = new Map<number, AbortController>();
/** Stops the pending nudge of a session (the user wrote in it, or it was closed). */
export const cancelNudge = (id: number) => {
  nudges.get(id)?.abort();
  nudges.delete(id);
};

/** Types a note into a resumed agent once its screen has settled after booting. Fire and forget; gives up quietly when
 *  the session is gone, was written to by the user meanwhile, or `cancelNudge` was called. */
export const nudge = async (id: number, text: string) => {
  cancelNudge(id);
  const ac = new AbortController();
  nudges.set(id, ac);
  const wait = (ms: number) =>
    new Promise<boolean>((done) => {
      const t = setTimeout(() => done(!ac.signal.aborted), ms);
      ac.signal.addEventListener("abort", () => (clearTimeout(t), done(false)), { once: true });
    });
  try {
    const start = Date.now();
    if (!(await wait(4000))) return;
    while (Date.now() - start < 90000) {
      const s = (await sessionsShared().catch(() => null))?.find((x) => x.id === id);
      if (ac.signal.aborted || !s || !s.running) return;
      if ((s.idle_ms ?? 0) >= 2500) break;
      if (!(await wait(1000))) return;
    }
    if (ac.signal.aborted) return;
    await rawWrite(id, text);
    if (!(await wait(400))) return;
    await rawWrite(id, "\r");
  } finally {
    if (nudges.get(id) === ac) nudges.delete(id);
  }
};

/** What an agent should be told about the restart: it was mid-task, and/or workers it started were stopped. */
const restartNote = (interrupted: boolean, lost: string[]) => {
  const parts: string[] = [];
  if (interrupted)
    parts.push(
      "Axelstra was restarted and this conversation was resumed. You were in the middle of a task: continue where you left off.",
    );
  if (lost.length) {
    parts.push(
      `Workers you started were stopped by the restart and are gone: ${lost.join(", ")}. Their worktrees keep what they had already done. Run \`axelstra ls\` and relaunch what is unfinished (a running \`axelstra flow\` stopped too: run it again with \`--resume\` and it skips what finished).`,
    );
  }
  return parts.length ? `[axelstra] ${parts.join(" ")}` : "";
};

export type RestoreResult = { restored: number; skipped: number; failed: string[] };
type PendingRec = {
  program: string;
  args?: string[];
  cwd?: string;
  label?: string;
  meta?: SessionMeta;
  env?: Record<string, string>;
};

const restoreOnce = async (): Promise<RestoreResult> => {
  const pending = await call<PendingRec[]>("session.pending");
  let restored = 0;
  let skipped = 0;
  const failed: string[] = [];
  // rules depend only on the reply style: read once per distinct level (the project's own, else the global one) for the whole batch
  const rulesByTerse = new Map<Terse | undefined, Promise<string | null>>();
  const rulesFor = (terse?: Terse) => {
    if (!rulesByTerse.has(terse))
      rulesByTerse.set(
        terse,
        getSettings(terse).then(
          (x) => x.rules,
          () => null,
        ),
      );
    return rulesByTerse.get(terse)!;
  };
  for (const r of pending) {
    const m = r.meta;
    try {
      if (m?.kind === "agent" && m.agent && m.project && m.worktree) {
        // a cleared / never used conversation has nothing to bring back: no tab, no process
        if (m.blank) {
          skipped++;
        } else {
          const terse = apiContext.terse(m.project);
          const rules = await rulesFor(terse);
          const id = await launchDefaultAgent(
            m.project,
            m.worktree,
            r.label ?? m.agent,
            m.agent,
            { key: m.key, resume: m.resume, note: m.note, lead: m.lead },
            { rules },
            terse,
          );
          const note = restartNote(!!m.interrupted, m.lost_workers ?? []);
          if (note) void nudge(id, note).catch((e) => console.debug("nudge", e));
          restored++;
        }
      } else {
        await spawnSession(r.program, r.args ?? [], r.label ?? undefined, r.cwd, m ?? undefined, r.env);
        restored++;
      }
    } catch {
      // usually a worktree that no longer exists
      failed.push(r.label ?? m?.worktree ?? r.cwd ?? r.program);
    }
    // this entry is dealt with either way; the others stay listed until their turn, so a crash midway loses nothing
    await call("session.pending_remove", { rec: r }).catch((e) => console.debug("session.pending_remove", e));
  }
  return { restored, skipped, failed };
};

/** One restore at a time: it runs on mount and again on `daemon-restarted`, and two overlapping runs would both read the
 *  same `session.pending` and start every agent twice. A call that arrives while one runs shares one trailing run (which
 *  sees only what the first left, since each entry is removed as it is dealt with). */
let restoring: Promise<RestoreResult> | null = null;
let restoreTrailing: Promise<RestoreResult> | null = null;
/** Brings back the sessions of a previous daemon run (reboot, installer): shells reopen, agents resume their conversation. */
export const restoreSessions = (): Promise<RestoreResult> => {
  if (!restoring) {
    const p: Promise<RestoreResult> = restoreOnce().finally(() => {
      if (restoring === p) restoring = null;
    });
    return (restoring = p);
  }
  return (restoreTrailing ??= restoring.then(
    () => ((restoreTrailing = null), restoreSessions()),
    () => ((restoreTrailing = null), restoreSessions()),
  ));
};

/** Change events for the selected worktree: the daemon watches the files and tells us, so git runs only when needed. */
type WatchHandlers = { onChanged: (worktree: string) => void; onSessions?: () => void; onProjects?: () => void };
let handlers: WatchHandlers | null = null;
let lastWorktree: string | null = null;
let wanted = false;
let watchReady: Promise<void> | null = null;
let channel: Channel<{ event?: string; worktree?: string }> | null = null;
let unlisteners: Promise<() => void>[] = [];

/** Events the daemon pushes on the watch stream besides the file, session and project ones: the app takes what changed
 *  when one arrives instead of asking on a timer. */
export type DaemonEvent = "janitor.events" | "triggers.events" | "budget.events" | "dispatch.changed";
const eventSubs = new Map<string, Set<() => void>>();
/** Runs `fn` each time the daemon announces `name` (and after the stream reconnects, in case one was missed); returns the unsubscribe. */
export const onDaemonEvent = (name: DaemonEvent, fn: () => void): (() => void) => {
  let set = eventSubs.get(name);
  if (!set) eventSubs.set(name, (set = new Set()));
  set.add(fn);
  return () => void set.delete(fn);
};
const fire = (name: string) => eventSubs.get(name)?.forEach((f) => f());

/** Drops the current Channel: its late messages go nowhere. (Tauri has no close for a Channel; the host replaces its end on the next watch_start.) */
const dropChannel = () => {
  if (channel) channel.onmessage = () => {};
  channel = null;
  watchReady = null;
};
const startWatch = () => {
  dropChannel();
  const ch = new Channel<{ event?: string; worktree?: string }>();
  channel = ch;
  // the handlers are read when a message arrives, so the newest ones from watchWorktree win
  ch.onmessage = (m) => {
    if (ch !== channel || !handlers) return;
    if (m?.event === "fs.changed" && m.worktree) handlers.onChanged(m.worktree);
    else if (m?.event === "session.changed") handlers.onSessions?.();
    else if (m?.event === "projects.changed") handlers.onProjects?.();
    else if (m?.event) fire(m.event);
  };
  return tauriInvoke<void>("watch_start", { onEvent: ch });
};
/** The watch connection died (Rust: "watch-ended") or the whole daemon was replaced ("daemon-restarted"): connect again for the same worktree.
 *  Events that arrive while a reconnection is already under way are the same event. */
let reconnecting = false;
const rewatch = () => {
  if (!wanted || !handlers || reconnecting) return;
  reconnecting = true;
  dropChannel();
  void watchWorktree(lastWorktree, handlers.onChanged, handlers.onSessions, handlers.onProjects).finally(() => {
    reconnecting = false;
    // whatever happened while the stream was down is not coming as an event: look once
    handlers?.onSessions?.();
    handlers?.onProjects?.();
    for (const name of eventSubs.keys()) fire(name);
  });
};
export const watchWorktree = async (
  worktree: string | null,
  onChanged: (worktree: string) => void,
  onSessions?: () => void,
  onProjects?: () => void,
) => {
  handlers = { onChanged, onSessions, onProjects };
  lastWorktree = worktree;
  wanted = true;
  if (!unlisteners.length) {
    unlisteners = ["watch-ended", "daemon-restarted"].map((ev) =>
      listen(ev, rewatch).catch((e) => {
        console.debug("listen", ev, e);
        return () => {};
      }),
    );
  }
  const ready = (watchReady ??= startWatch());
  try {
    await ready;
    await tauriInvoke("watch_set", { worktree: lastWorktree });
  } catch (e) {
    // daemon restarted or never reachable: reconnect on the next selection
    console.debug("watch", e);
    if (watchReady === ready) watchReady = null;
  }
};
/** Ends the watch for good: no listeners left, the Channel is dropped (app teardown, tests). */
export const stopWatch = () => {
  wanted = false;
  handlers = null;
  reconnecting = false;
  dropChannel();
  const l = unlisteners;
  unlisteners = [];
  for (const u of l) void u.then((f) => f());
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
      {
        cols: Math.max(80, Math.floor((window.innerWidth * 0.7 - 16) / 7.8)),
        rows: Math.max(24, Math.floor((window.innerHeight - 110) / 15)),
      };

export const spawnSession = (
  program: string,
  args: string[] = [],
  label?: string,
  cwd?: string,
  meta?: SessionMeta,
  env?: Record<string, string>,
) => call<{ id: number }>("session.spawn", { program, args, label, cwd, meta, env, ...startSize() });

/** the task file of an agent: its text, path and name */
export const notesRead = (session: number) =>
  call<{ text: string; path: string; name: string }>("notes.read", { session });
/** copies the task file into the worktree's .axelstra folder; refuses to replace a file unless `overwrite` */
export const notesExport = (session: number, overwrite = false) =>
  call<{ path: string }>("notes.export", { session, overwrite });
/** worktree path -> the other worktrees that edit some of the same files */
export const dispatchOverlaps = () => callBg<Record<string, string[]>>("dispatch.overlaps");
export const dispatchOverlapFiles = () => callBg<OverlapFiles>("dispatch.overlap_files");
/** the name of a worktree folder/branch as the daemon renames it */
export const worktreeRename = (path: string, name: string) =>
  call<{ path?: string }>("worktree.rename", { path, name });

export const killSession = (id: number) => (cancelNudge(id), call("session.kill", { id }));
export const removeSession = (id: number) => (cancelNudge(id), call("session.remove", { id }));
export const resizeSession = (id: number, cols: number, rows: number) => call("session.resize", { id, cols, rows });
const rawWrite = (id: number, data: string) =>
  invoke("session_write", { id, data }).catch((e) => Promise.reject(toAppError("session_write", e)));
/** Text typed or pasted by the user (or a feature acting for them): a pending nudge into this session is dropped, it would interleave. */
export const writeSession = (id: number, data: string) => (cancelNudge(id), rawWrite(id, data));

export const listProjects = () => call<Project[]>("project.list");
/** A host command that opens a native dialog first: no timeout, since the user may browse for a long time. */
const nativePick = <T>(method: string, cmd: string, title: string) =>
  invoke<T>(cmd, { title }).catch((e) => Promise.reject(toAppError(method, e)));
/** Native folder dialog on the Rust side, then `project.add` with the chosen folder; null when cancelled.
 *  The webview never supplies the path (`project.add` is not in the host's allow-list). */
export const pickAndAddProject = (title: string) =>
  nativePick<Project | null>("project.add", "project_add_pick", title);
export const projectLogo = (id: string) =>
  call<{ data_url: string | null }>("project.logo", { id }).then((r) => r.data_url);
/** per-project integration settings; send only what changed */
export const setProjectIntegrate = (id: string, s: { auto?: boolean; push?: boolean }) =>
  call("project.integrate", { id, ...s });
export type IntegrateResult = {
  integrated: boolean;
  into: string;
  how: "ff" | "merge" | "none";
  removed: boolean;
  pushed: boolean | null;
};
/** integrates a worktree's branch into the default branch; fails with a coded error when it cannot */
export const integrateWorktree = (project: string, path: string) =>
  call<IntegrateResult>("worktree.integrate", { project, path });
export const setProjectSetup = (id: string, setup: string) => call("project.setup", { id, setup });
export const detectPackageManagers = (id: string) =>
  call<{ installed: string[]; suggested: string | null; package_json?: boolean }>("pm.detect", { id });
export type DispatchMeta = Record<
  string,
  { title?: string; archived?: boolean; requests?: string[]; updated?: number }
>;
export const removeProject = (id: string) => call("project.remove", { id });
export type EnvSkip = { path: string; reason: string };
/** worktree.create also says what .worktreeinclude copied into the new worktree and what it left out */
export type WorktreeCreated = Worktree & { env_copied?: string[]; env_skipped?: EnvSkip[] };
export const createWorktree = (project: string, name: string, parent?: string) =>
  call<WorktreeCreated>("worktree.create", { project, name, parent });
/** groups a worktree under another one of the same project; null puts it back at the top level */
export const worktreeSetParent = (project: string, path: string, parent: string | null) =>
  call("worktree.set_parent", { project, path, parent });
/** a task file (report) of an agent that worked in a worktree */
export type WorktreeNote = { name: string; status: "ok" | "nota" | "aberto"; excerpt: string; modified: number };
export const notesWorktree = (project: string, worktree: string) =>
  call<{ items: WorktreeNote[] }>("notes.worktree", { project, worktree }).then((r) => r.items);
export const notesReadFile = (project: string, name: string) =>
  call<{ text: string; path: string; name: string }>("notes.read_file", { project, name });
/** the ignored files .worktreeinclude would copy into the next worktree of the project */
export type EnvPreview = { has_file: boolean; files: string[]; skipped: EnvSkip[] };
export const projectEnvPreview = (project: string) => call<EnvPreview>("project.env_preview", { project });
export type GhItem = {
  number: number;
  title: string;
  author: string;
  url: string;
  state: string;
  draft: boolean;
  branch: string;
  checks: string;
  review: string;
};
export const ghList = (project: string, kind: "pr" | "issue") =>
  call<{ items: GhItem[] }>("gh.list", { project, kind }).then((r) => r.items);
export const ghListBg = (project: string, kind: "pr" | "issue") =>
  callBg<{ items: GhItem[] }>("gh.list", { project, kind }).then((r) => r.items);
/** An inline review comment of a PR. Every text field was written by a reviewer: untrusted. */
export type ReviewComment = {
  id: number;
  path: string;
  line: number;
  body: string;
  author: string;
  diff_hunk: string;
  created: string;
  url: string;
  in_reply_to: number;
  outdated: boolean;
  pr: number;
};
/** A review as a whole (verdict plus its text); reviews without text are left out by the daemon. */
export type ReviewSummary = {
  id: number;
  author: string;
  body: string;
  state: "approved" | "changes" | "commented" | "dismissed" | "pending";
  created: string;
  url: string;
};
export const ghComments = (project: string, number: number) =>
  call<{ comments: ReviewComment[]; reviews: ReviewSummary[] }>("gh.comments", { project, number });
/** `dryRun` only builds the text that would be sent (from GitHub's current version of the comment); nothing is delivered. */
export const ghSendComment = (worktree: string, project: string, number: number, commentId: number, dryRun: boolean) =>
  call<{ text: string; sent: boolean; session?: number }>("gh.send_comment", {
    worktree,
    project,
    number,
    comment_id: commentId,
    dry_run: dryRun,
  });
export const worktreeFromPr = (project: string, number: number) =>
  call<Worktree>("worktree.from_pr", { project, number });
export const listPorts = (worktree: string) =>
  call<{ ports: { port: number; pid: number }[] }>("ports.list", { worktree }).then((r) => r.ports);
export type Schedule =
  | { kind: "interval"; minutes: number }
  | { kind: "daily"; time: string }
  | { kind: "weekly"; day: number; time: string };
export type Automation = {
  id: string;
  name: string;
  project: string;
  agent: string;
  prompt: string;
  schedule: Schedule;
  enabled: boolean;
  last_run: number;
};
export const listAutomations = () => call<{ automations: Automation[] }>("automation.list").then((r) => r.automations);
export const saveAutomation = (a: Automation) => call<Automation>("automation.save", { ...a });
export const deleteAutomation = (id: string) => call("automation.delete", { id });
export const runAutomation = (id: string) => call<{ session: number; worktree: string }>("automation.run", { id });
export const openInEditor = (worktree: string, app: "code") => call("open.editor", { worktree, app });
export const worktreeRebase = (project: string, path: string) =>
  call<RebaseResult>("worktree.rebase", { project, path });
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
export const fsGrep = (worktree: string, query: string) =>
  call<{ matches: GrepHit[] }>("fs.grep", { worktree, query }).then((r) => r.matches);
export const fsCreate = (worktree: string, path: string, dir: boolean) => call("fs.create", { worktree, path, dir });
export const fsRename = (worktree: string, path: string, to: string) => call("fs.rename", { worktree, path, to });
export const fsDelete = (worktree: string, path: string) => call("fs.delete", { worktree, path });
export const fsRead = (worktree: string, path: string) => call<Opened>("fs.read", { worktree, path });
export const fsStat = (worktree: string, path: string) =>
  call<{ hash: string | null }>("fs.stat", { worktree, path }).then((r) => r.hash);
export const fsWrite = (worktree: string, path: string, text: string, eol: string, bom: boolean, base_hash?: string) =>
  call<Saved>("fs.write", { worktree, path, text, eol, bom, base_hash });

export const gitCommit = (worktree: string, message: string) =>
  call<{ sha: string }>("git.commit", { worktree, message });
/** Commit message the user's `claude` CLI proposes for the pending changes (nothing is staged or committed). */
export const gitSuggest = (worktree: string) =>
  call<{ message: string }>("git.suggest", { worktree }).then((r) => r.message);
/** Title and markdown description the user's `claude` CLI proposes for a PR of this worktree's branch against `base` (default branch when omitted). */
export const gitSuggestPr = (worktree: string, base?: string) =>
  call<{ title: string; body: string }>("git.suggest_pr", { worktree, base });
export const gitPush = (worktree: string) => call<{ output: string }>("git.push", { worktree });
export const gitPr = (worktree: string, title: string, body: string, base?: string) =>
  call<{ url: string }>("git.pr", { worktree, title, body, base }).then((r) => r.url);
export type CheckItem = { name: string; state: "pass" | "fail" | "pending" | "skipped"; link: string };
/** CI and merge state of the PR behind a worktree's branch; `problem` is set when there is none or gh is unusable. */
export type ShipStatus = {
  problem: "" | "nopr" | "missing" | "auth" | "error";
  detail: string;
  number: number;
  title: string;
  url: string;
  pr_state: "open" | "merged" | "closed";
  draft: boolean;
  ci: "pass" | "fail" | "pending" | "none";
  checks: CheckItem[];
  review: "approved" | "changes" | "review" | "";
  mergeable: "yes" | "no" | "unknown";
  merge_state: string;
  can_merge: boolean;
};
export const gitTimeline = (worktree: string, project: string, ci = false) =>
  callBg<Timeline>("git.timeline", { worktree, project, ci });
export const gitChecks = (worktree: string) => callBg<ShipStatus>("git.checks", { worktree });
export const gitMerge = (worktree: string, auto: boolean, deleteBranch: boolean) =>
  call<{ output: string }>("git.merge", { worktree, auto, delete_branch: deleteBranch }).then((r) => r.output);

export type KCard = {
  meta: {
    id: string;
    title: string;
    status: string;
    agent: string;
    depends_on: string[];
    priority: number;
    acceptance?: string | null;
  };
  body: string;
};
export type Board = {
  cards: KCard[];
  broken: { file: string; error: string; error_code?: string; error_params?: Record<string, string> }[];
};

export const kanbanList = (project: string) => call<Board>("kanban.list", { project });
export const kanbanSave = (project: string, card: Record<string, unknown>) =>
  call<KCard>("kanban.save", { project, ...card });
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
  /** message code of `error` (src/msg), and of each failed / needs-review reason by card id */
  error_code?: string;
  error_params?: Record<string, string>;
  reason_codes?: Record<string, { code: string; params: Record<string, string> }>;
};
/** `pin` (from orchTrustInfo) lets `bypass*` cards run, but only if the todo cards still hash to what the user confirmed */
export const orchStart = (project: string, max = 3, trustAcceptance = false, pin?: string) =>
  call<OrchRun>("orch.start", { project, max, trust_acceptance: trustAcceptance, ...(pin ? { pin } : {}) });

/** What the user must confirm before a run: each todo card's agent, permission mode and acceptance command (mirrors orch::TrustInfo). */
export type OrchTrustCard = {
  id: string;
  agent: string;
  permission_mode: string;
  bypass: boolean;
  acceptance: string | null;
};
export type OrchTrust = { pin: string; cards: OrchTrustCard[] };
/** Trust info of the project's todo cards for the Kanban confirm; null while the running daemon has no `orch.trust_info`. */
export const orchTrustInfo = (project: string) => callOr<OrchTrust | null>("orch.trust_info", { project }, null);
/** merges a needs-review card again after the user fixed its worktree */
export const orchRetry = (project: string, id: string, trustAcceptance = false) =>
  call("orch.retry", { project, id, trust_acceptance: trustAcceptance });
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
export const worktreeStatus = (paths: string[]) =>
  callBg<{ status: WtStatus[] }>("worktree.status", { paths }).then((r) => r.status);

/** counts per project for the status bar (cached by the daemon for about 10 s) */
export const worktreeOverview = () =>
  callBg<{ projects: import("./statusGroups").OverviewProject[] }>("worktree.overview").then((r) => r.projects);
/** removes the worktrees whose work is merged (the janitor's sweep) */
export type SweepItem = {
  path: string;
  branch: string;
  removed: boolean;
  kind?: string;
  why?: string;
  warning?: string;
};
export const worktreeSweep = () => call<{ swept: SweepItem[] }>("worktree.sweep", {}).then((r) => r.swept);

/** `project` is Gemini's project hash (its logs keep no folder); `geminiPathVariants` + sha-256 map it back to a worktree. */
export type AgentHit = {
  agent: "claude" | "codex" | "gemini";
  session: string;
  cwd: string;
  ts: number;
  snippet: string;
  project?: string;
};
/** Ways Gemini may have spelled a worktree path before hashing it (mirrors gemini_hashes in crates/daemon/src/agentlog.rs). */
export const geminiPathVariants = (path: string) => {
  const t = path.replace(/^\\\\\?\\/, "").replace(/[\\/]+$/, "");
  return [...new Set([path, t, t.replace(/\//g, "\\"), t.replace(/\\/g, "/")])];
};
/** What launchDefaultAgent gets as `resume` to reopen a hit's conversation; null when the agent cannot resume it. */
export const hitResume = (h: Pick<AgentHit, "agent" | "session">) =>
  h.agent === "claude" ? h.session : h.agent === "gemini" ? `gemini:${h.session}` : null;
export const agentSearch = (query: string, max = 30) =>
  callBg<{ hits: AgentHit[] }>("agent.search", { query, max }).then((r) => r.hits);

export type TokenUsage = {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  messages: number;
  context: number;
};
export const agentUsage = (cwd: string, session?: string) =>
  callBg<{ usage: TokenUsage | null }>("agent.usage", { cwd, session }).then((r) => r.usage);
export const worktreeUsage = (worktrees: string[]) =>
  callBg<{ usage: WtUsage[] }>("agent.worktree_usage", { worktrees }).then((r) => r.usage);

export const computerStatus = () => call<{ enabled: boolean }>("computer.status").then((r) => r.enabled);
export const computerEnable = (on: boolean) =>
  call<{ enabled: boolean }>("computer.enable", { on }).then((r) => r.enabled);

export type WorkerLimits = { max_workers: number; workers: number };
export type SessionRes = { id: number; mem: number; cpu_100ns: number };
export const sessionResources = () => callBg<{ sessions: SessionRes[]; cores: number }>("session.resources");

/** tasks of running `axelstra flow` plans: queued (waiting for its turn), running, done, failed, blocked */
export type FlowTask = { id: string; worktree: string; state: string };
export const flowTasks = () => callBg<{ tasks: FlowTask[] }>("flow.list").then((r) => r.tasks);

export const workerLimits = (max?: number) =>
  call<WorkerLimits>("orch.limits", max === undefined ? {} : { max_workers: max });

/** What Claude Code's status line last reported for an agent tab (context window, 5 h and weekly limits). */
export type ClaudeStatus = {
  context_window?: { context_window_size?: number; used_percentage?: number };
  rate_limits?: {
    five_hour?: { used_percentage?: number; resets_at?: number };
    seven_day?: { used_percentage?: number; resets_at?: number };
  };
};
export const agentStatus = (key: string) =>
  callBg<{ status: ClaudeStatus | null }>("agent.status", { key }).then((r) => r.status);

/** One entry of the daemon's agents table; `installed` says whether its executable is on the PATH. */
export type AgentInfo = {
  id: string;
  name: string;
  installed: boolean;
  path: string | null;
  install: string;
  can_resume: boolean;
  interactive_prompt: boolean;
};
/** Every agent the app knows (claude, codex, gemini) with its installed state. */
export const agentsList = () => callBg<{ agents: AgentInfo[] }>("agents.list").then((r) => r.agents);

/** Select options for the agent pickers: the installed agents, plus `current` when it is not (so a saved choice never vanishes). */
export const agentOptions = (list: AgentInfo[], current?: string) => {
  const mark = (id: string) => (["claude", "codex", "gemini"].includes(id) ? id : undefined);
  const out: { value: string; label: string; icon?: string }[] = list
    .filter((a) => a.installed)
    .map((a) => ({ value: a.id, label: a.name, icon: mark(a.id) }));
  const cur = current && list.find((a) => a.id === current || (current === "chatgpt" && a.id === "codex"));
  if (current && !out.some((o) => o.value === current || (current === "chatgpt" && o.value === "codex"))) {
    out.push({ value: current, label: cur ? L.agent.notInstalled(cur.name) : L.agent.unknownInList(current) });
  }
  return out;
};

/** warning (in the UI language) for a chosen agent that cannot run, with the official install command; empty when it is fine. */
export const agentWarning = (list: AgentInfo[], id: string) => {
  const a = list.find((x) => x.id === id || (id === "chatgpt" && x.id === "codex"));
  if (!a) return list.length ? L.agent.unknown(id) : "";
  return a.installed ? "" : L.agent.notFound(a.name, a.install);
};

export type CodexWindow = {
  used_percent?: number;
  window_minutes?: number;
  resets_at?: number;
  resets_in_seconds?: number;
};
export type CodexLimits = { primary?: CodexWindow; secondary?: CodexWindow; as_of?: number };
export const codexLimits = () => callBg<{ limits: CodexLimits | null }>("codex.limits").then((r) => r.limits);

// ---- docker ----
export type DockerStatus = { installed: boolean; running: boolean; version: string; error: string };
export type DockerPort = { host: number | null; container: number; proto: string };
export type DockerContainer = {
  id: string;
  name: string;
  image: string;
  state: string;
  status: string;
  ports: DockerPort[];
  created: string;
  project: string;
  working_dir: string;
  service: string;
  volumes: string[];
  networks: string[];
};
export type DockerProject = { name: string; working_dir: string; running: number; total: number };
export type DockerCompose = { file: string | null; project: string; services: DockerContainer[] };
export type DockerStat = { id: string; name: string; cpu: number; mem: string; mem_pct: number };
export type DockerImage = {
  id: string;
  tags: string[];
  size: string;
  created: string;
  containers: number;
  dangling: boolean;
};
export type DockerVolume = { name: string; driver: string; mountpoint: string; used_by: string[] };
export type DockerNetwork = { id: string; name: string; driver: string; scope: string; containers: string[] };
export type DockerInspect = {
  command: string;
  workdir: string;
  restart: string;
  env: [string, string][];
  mounts: [string, string, string, boolean][];
  networks: [string, string][];
};
export type DockerFsEntry = { name: string; dir: boolean };
export type DockerAction =
  | "container.start"
  | "container.stop"
  | "container.restart"
  | "container.pause"
  | "container.unpause"
  | "container.kill"
  | "container.remove"
  | "image.remove"
  | "volume.remove"
  | "prune.containers"
  | "prune.images"
  | "prune.volumes"
  | "compose.up"
  | "compose.down";

// queries go over the second connection: the 3 s poll must not hold up interactive calls
export const dockerStatus = () => callBg<DockerStatus>("docker.status");
export const dockerContainers = (worktree?: string | null) =>
  callBg<{ projects: DockerProject[]; containers: DockerContainer[]; compose: DockerCompose | null }>(
    "docker.containers",
    worktree ? { worktree } : {},
  );
export const dockerStats = () => callBg<{ stats: DockerStat[] }>("docker.stats").then((r) => r.stats);
export const dockerImages = () => callBg<{ images: DockerImage[] }>("docker.images").then((r) => r.images);
export const dockerVolumes = () => callBg<{ volumes: DockerVolume[] }>("docker.volumes").then((r) => r.volumes);
export const dockerNetworks = () => callBg<{ networks: DockerNetwork[] }>("docker.networks").then((r) => r.networks);
export const dockerInspect = (id: string) => callBg<DockerInspect>("docker.inspect", { id });
export const dockerLogs = (id: string, tail = 500) =>
  callBg<{ text: string }>("docker.logs", { id, tail }).then((r) => r.text);
export const dockerFsList = (id: string, path: string) =>
  callBg<{ entries: DockerFsEntry[] }>("docker.fs_list", { id, path }).then((r) => r.entries);
export const dockerFsRead = (id: string, path: string) =>
  callBg<{ text: string }>("docker.fs_read", { id, path }).then((r) => r.text);
export const dockerAction = (
  action: DockerAction,
  o: { id?: string; force?: boolean; volumes?: boolean; worktree?: string } = {},
) => call<{ output: string }>("docker.action", { action, ...o }).then((r) => r.output);
/** Logs and shell are sessions the daemon starts with a fixed command in the worktree; the answer is the session id. */
export const dockerOpenLogs = (worktree: string, id: string, project?: string) =>
  call<{ id: number }>("docker.open_logs", { worktree, id, project }).then((r) => r.id);
export const dockerOpenShell = (worktree: string, id: string, project?: string) =>
  call<{ id: number }>("docker.open_shell", { worktree, id, project }).then((r) => r.id);

/** Hooks that report agent state (needs input, done) to Axelstra, kept in the agent's own settings (~/.claude or ~/.gemini). */
export type HookAgent = "claude" | "gemini";
export const HOOK_AGENTS: readonly HookAgent[] = ["claude", "gemini"];
export const hookSettingsPath = (agent: HookAgent) =>
  agent === "gemini" ? "~/.gemini/settings.json" : "~/.claude/settings.json";
export type HooksStatus = { installed: boolean; path: string; events?: string[]; problem?: string | null };
export type HooksInstall = { changed: boolean; path: string; backup: string | null };
export const hooksStatus = (agent: HookAgent = "claude") => call<HooksStatus>("hooks.status", { agent });
export const hooksInstall = (agent: HookAgent = "claude") => call<HooksInstall>("hooks.install", { agent });
/** Hook agents to show: claude always, the others only unless the agent list says they are not installed (unknown list: show all). */
export const hookAgentsToShow = (list: AgentInfo[] | undefined): HookAgent[] =>
  HOOK_AGENTS.filter(
    (a) => a === "claude" || !list || list.length === 0 || list.some((x) => x.id === a && x.installed),
  );

// ---- daemon methods newer than some running daemons: each degrades (null / empty) on "unknown method" ----
/** Messages the daemon's store recorded (a corrupt file it recovered from, ...), taken once. Empty on an older daemon. */
export const storeNotices = () =>
  callOr<{ notices: string[]; items?: { text: string; code?: string; params?: Record<string, unknown> }[] }>(
    "notices.take",
    {},
    { notices: [] },
  ).then(
    // `items` pairs each English line with its message code, which is shown in the UI language
    (r) => (r.items ? r.items.map((i) => coded(i.code, i.params, i.text)) : r.notices),
  );
/** One command line tool Axelstra uses, as the daemon finds it on its PATH. */
export type EnvTool = { id: string; found: boolean; path: string | null; version: string | null };
/** git, claude and gh with their versions; null on a daemon without `env.tools`. */
export const envTools = () => callOr<{ tools: EnvTool[] } | null>("env.tools", {}, null).then((r) => r?.tools ?? null);
/** Copies the daemon's state files (settings, projects, sessions, automations; never the token) into a new backup folder; null on a daemon without it. */
export const backupExport = () => callOr<{ path: string } | null>("backup.export", {}, null);
/** Native folder dialog for the backup folder, then `backup.restore` on it. `undefined` = cancelled; null = the daemon is too old. */
export const pickAndRestoreBackup = (title: string): Promise<{ restored: string[] } | null | undefined> =>
  nativePick<{ restored: string[] } | null>("backup.restore", "backup_restore_pick", title).then(
    (r) => (r === null ? undefined : r),
    (e) => (isUnknownMethod(e) ? null : Promise.reject(e)),
  );

/** A token limit; a missing `daily` / `weekly` means no limit. `auto_pause` stays off unless the user turns it on. */
export type BudgetLimit = { daily?: number; weekly?: number; auto_pause?: boolean };
/** `default` applies to every worktree without an entry of its own in `worktrees` (keyed by path). */
export type Budgets = { default?: BudgetLimit; worktrees?: Record<string, BudgetLimit> };
export type BudgetLevel = "ok" | "warn" | "over";
export type BudgetRow = {
  worktree: string;
  period: "daily" | "weekly";
  used: number;
  limit: number;
  pct: number;
  level: BudgetLevel;
  auto_pause: boolean;
};
export type BudgetEvent = {
  worktree: string;
  period: "daily" | "weekly";
  level: "warn" | "over";
  used: number;
  limit: number;
  pct: number;
  paused: number[];
  at: number;
};
/** Spending against the limits for the given worktrees (one row per worktree and period that has a limit); empty on an older daemon. */
export const budgetStatus = (worktrees: string[]) =>
  callBg<{ budgets: BudgetRow[] }>("budget.status", { worktrees })
    .catch((e) => (isUnknownMethod(e) ? { budgets: [] as BudgetRow[] } : Promise.reject(e)))
    .then((r) => r.budgets ?? []);
/** Evaluates the limits now (the daemon also does it every two minutes); the rows after the check. */
export const budgetCheck = () =>
  callOr<{ budgets: BudgetRow[] }>("budget.check", {}, { budgets: [] }).then((r) => r.budgets ?? []);
/** Level crossings since the last call (take semantics); an older daemon has none. */
export const budgetEvents = () =>
  callOr<{ events: BudgetEvent[] }>("budget.events", {}, { events: [] }).then((r) => r.events ?? []);
// ---- workspace export / import: files and folders come from native dialogs held by the host, never from the page ----
type WsExported = { path: string; summary: { projects: number; automations: number; cards: number } };
/** A host command that opens a native dialog first (no timeout), with extra arguments. */
const hostCmd = <T>(method: string, cmd: string, args: Record<string, unknown> = {}) =>
  invoke<T>(cmd, args).catch((e) => Promise.reject(toAppError(method, e)));
/** Save dialog, then `workspace.export`. `undefined` = cancelled; null = the daemon is too old. */
export const pickAndExportWorkspace = (title: string): Promise<WsExported | null | undefined> =>
  hostCmd<WsExported | null>("workspace.export", "workspace_export_pick", { title }).then(
    (r) => r ?? undefined,
    (e) => (isUnknownMethod(e) ? null : Promise.reject(e)),
  );
/** Open dialog for a workspace file, then its preview. `undefined` = cancelled; null = the daemon is too old. */
export const pickWorkspaceFile = (title: string): Promise<WorkspacePreview | null | undefined> =>
  hostCmd<WorkspacePreview | null>("workspace.import_preview", "workspace_import_pick", { title }).then(
    (r) => r ?? undefined,
    (e) => (isUnknownMethod(e) ? null : Promise.reject(e)),
  );
/** Folder dialog for where the projects under `old` (a path of the file) live here; the new preview, or undefined when cancelled. */
export const mapWorkspaceFolder = (title: string, old: string): Promise<WorkspacePreview | undefined> =>
  hostCmd<WorkspacePreview | null>("workspace.import_preview", "workspace_map_pick", { title, old }).then(
    (r) => r ?? undefined,
  );
export const clearWorkspaceMap = () => hostCmd<WorkspacePreview>("workspace.import_preview", "workspace_map_clear");
/** `workspace.import` of the file picked before, with the folder map; the page only chooses how. */
export const applyWorkspaceImport = (mode: WorkspaceMode, conflicts: WorkspaceConflicts) =>
  hostCmd<WorkspaceOutcome>("workspace.import", "workspace_import_apply", { mode, conflicts });
/** The dialog was closed: the host forgets the picked file. */
export const cancelWorkspaceImport = () =>
  hostCmd<void>("workspace.import", "workspace_import_cancel").catch((e) =>
    console.debug("workspace_import_cancel", e),
  );

export type JanitorEvent = {
  project: string;
  path: string;
  branch: string;
  into: string;
  why: string;
  at: number;
  /** absent = "removed" (an older daemon) */
  kind?: "removed" | "integrated" | "empty" | "attention";
  how?: "ff" | "merge";
  pushed?: boolean | null;
  code?: string;
  params?: Record<string, unknown>;
  text?: string;
};
/** Worktrees the automatic sweep removed since the last call (take semantics); an older daemon has none. */
export const janitorEvents = () =>
  callOr<{ events: JanitorEvent[] }>("janitor.events", {}, { events: [] }).then((r) => r.events ?? []);

// ---- CI-fix trigger (daemon: triggers.*) ----
export type TriggerFiring = {
  ts: number;
  worktree: string;
  sha: string;
  checks: string[];
  kind: "fired" | "limited" | "failed";
  detail: string;
};
export type TriggerLimits = {
  cooldown_secs: number;
  max_per_hour: number;
  poll_secs: number;
  cooldown_range: [number, number];
  max_per_hour_range: [number, number];
};
export type TriggerState = {
  project: string;
  ci_fix: boolean;
  /** epoch seconds of the user's first confirmation; null until then */
  confirmed_at: number | null;
  limits: TriggerLimits;
  history: TriggerFiring[];
  agent: string;
  confirm_word: string;
  template: string;
  example: string;
};
export type TriggerPreview = {
  project: string;
  prompt: string;
  template: string;
  limits: TriggerLimits;
  agent: string;
  confirm_word: string;
  never_main: boolean;
  never_pr_worktrees: boolean;
  audit_method: string;
};
export type TriggerEvent = {
  kind: "ci_fix" | "ci_fix_limited" | "ci_fix_failed";
  project: string;
  worktree: string;
  branch: string;
  sha?: string;
  detail?: string;
  detail_code?: string;
  detail_params?: Record<string, unknown>;
};
/** State, limits and prompt template of a project's CI-fix trigger; null on a daemon without `triggers.*`. */
export const triggersGet = (project: string) => callOr<TriggerState | null>("triggers.get", { project }, null);
/** What enabling would do (the prompt with an example failure, the limits); null on an older daemon. */
export const triggersPreview = (project: string) =>
  callOr<TriggerPreview | null>("triggers.preview", { project }, null);
/** Turns the trigger on/off and/or changes its limits. The first enabling needs `confirm` = the state's `confirm_word`. */
export const triggersSet = (
  project: string,
  patch: { ci_fix?: boolean; confirm?: string; cooldown_secs?: number; max_per_hour?: number },
) => call<TriggerState>("triggers.set", { project, ...patch });
/** What the trigger did since the last call (take semantics); an older daemon has none. */
export const triggersEvents = () =>
  callOr<{ events: TriggerEvent[] }>("triggers.events", {}, { events: [] }).then((r) => r.events ?? []);

// ---- remote hosts (M7b): the registry lives in the Tauri host, not in the daemon ----
export type HostState = "idle" | "connecting" | "ok" | "lost" | "auth";
export type HostInfo = {
  id: string;
  name: string;
  spec: string;
  slot: number;
  state: HostState;
  error?: string;
  /** message code of `error` when it is a single coded message */ error_code?: string;
  error_params?: Record<string, unknown>;
  proto?: number;
  build?: string;
};
export type HostInfoReport = {
  os: string;
  home: string;
  user: string;
  hostname: string;
  shell: string;
  sep: string;
  proto: number;
  build: string;
};
export type BrowseEntry = { name: string; git: boolean };
export type BrowseResult = {
  path: string;
  parent: string | null;
  home: string;
  entries: BrowseEntry[];
  truncated?: boolean;
};
export const hostsList = () => hostCmd<HostInfo[]>("hosts_list", "hosts_list");
export const hostAdd = (name: string, spec: string, bin?: string) =>
  hostCmd<HostInfo>("host_add", "host_add", { name, spec, ...(bin ? { bin } : {}) });
export const hostRemove = (id: string) => hostCmd<void>("host_remove", "host_remove", { id });
export const hostReconnect = (id: string) => hostCmd<HostInfo>("host_reconnect", "host_reconnect", { id });
/** the host shows its own confirmation and checks the path exists on the remote */
export const projectAddRemote = (host: string, path: string) =>
  hostCmd<Project>("project_add_remote", "project_add_remote", { host, path });
export const hostInfo = (host: string) => call<HostInfoReport>("host.info", { host });
export const hostBrowse = (host: string, path?: string) =>
  call<BrowseResult>("host.browse", path ? { host, path } : { host });
