import { createSignal } from "solid-js";
import { askSave } from "../Confirm";
import type { Commit, SessionInfo, Terse } from "../api";
import { norm } from "../paths";
import { createPersisted, isRecord, isStringArray, readJson, readString, writeJson, writeString } from "../persist";

export * from "./layout";

/** CLI agents that can be launched; the key is what the UI shows and stores */
export const AGENTS: Record<string, string> = { claude: "claude", codex: "codex", chatgpt: "codex" };
/** "chatgpt" was this agent's name in earlier versions: saved choices still say it */
export const canonAgent = (a: string) => (a === "chatgpt" ? "codex" : a);
/** names of the agents the pickers list: only the agent CLIs found on the PATH (empty until the daemon answers) */
export const [agentKeys, setAgentKeys] = createSignal<string[]>([]);
/** `ids` are the daemon's ids of the installed agents; without the list, the two built-ins */
export const setInstalledAgents = (ids: string[] | null) => {
  const keys = ids ?? ["claude", "codex"];
  for (const k of keys) AGENTS[k] ??= k;
  setAgentKeys(keys);
  // the saved default may not be installed here
  if (keys.length && !keys.includes(agent())) setAgentSig(keys[0]);
};
const savedAgent = readString("axelstra.agent");
const [agent, setAgentSig] = createSignal<string>(savedAgent && AGENTS[savedAgent] ? canonAgent(savedAgent) : "claude");
export const defaultAgent = agent;
export const setDefaultAgent = (a: string) => {
  if (!AGENTS[a]) return;
  setAgentSig(a);
  writeString("axelstra.agent", a);
};

/** Settings that belong to one project, remembered in the browser storage next to the global ones. */
export type ProjectPrefs = { agent?: string; terse?: Terse };
/** the saved preferences, keeping only the entries that have the shape of ProjectPrefs */
export const parsePrefs = (v: unknown): Record<string, ProjectPrefs> => {
  const out: Record<string, ProjectPrefs> = {};
  if (!isRecord(v)) return out;
  for (const [id, p] of Object.entries(v)) {
    if (!isRecord(p)) continue;
    const e: ProjectPrefs = {};
    if (typeof p.agent === "string") e.agent = p.agent;
    if (p.terse === "off" || p.terse === "short" || p.terse === "tiny") e.terse = p.terse;
    if (Object.keys(e).length) out[id] = e;
  }
  return out;
};
const loadPrefs = (): Record<string, ProjectPrefs> => parsePrefs(readJson("axelstra.projectPrefs", isRecord, {}));
const [projectPrefs, setProjectPrefsSig] = createSignal<Record<string, ProjectPrefs>>(loadPrefs());
/** the agent a project opens by default: its own choice, else the global one */
export const projectAgent = (id?: string | null) => {
  const a = id ? projectPrefs()[id]?.agent : undefined;
  return a && AGENTS[a] ? canonAgent(a) : defaultAgent();
};
export const projectPrefsOf = (id: string): ProjectPrefs => projectPrefs()[id] ?? {};
/** reply style chosen for one project; undefined = follow the global setting */
export const projectTerse = (id?: string | null): Terse | undefined => (id ? projectPrefs()[id]?.terse : undefined);
/** changes one project's preference; `undefined` for a field removes it (back to the global value) */
export const setProjectPref = (id: string, patch: ProjectPrefs) => {
  const next = { ...projectPrefs() };
  const merged: ProjectPrefs = { ...next[id], ...patch };
  for (const k of Object.keys(merged) as (keyof ProjectPrefs)[]) if (merged[k] === undefined) delete merged[k];
  if (Object.keys(merged).length) next[id] = merged;
  else delete next[id];
  setProjectPrefsSig(next);
  writeJson("axelstra.projectPrefs", next);
};

/** Modal asking which agent to open. Resolves with the agent key, "shell" for a plain terminal, or null if dismissed. */
export const [agentPrompt, setAgentPrompt] = createSignal<((a: string | null) => void) | null>(null);
export const pickAgent = () =>
  new Promise<string | null>((resolve) => {
    setAgentPrompt(() => (a: string | null) => {
      setAgentPrompt(null);
      resolve(a);
    });
  });

/** unsaved text in the open editor; anything that would close it asks first */
export const [editorDirty, setEditorDirty] = createSignal(false);
/** App registers how a session tab closes (with its confirmation), so the sidebar can close an agent the same way */
let sessionCloser: ((s: SessionInfo) => Promise<void>) | null = null;
export const setSessionCloser = (f: typeof sessionCloser) => {
  sessionCloser = f;
};
export const closeSessionTab = (s: SessionInfo) => sessionCloser?.(s);

/** the open editor registers how to save itself, so closing can offer "Guardar" */
let editorSaver: (() => Promise<boolean>) | null = null;
export const setEditorSaver = (f: (() => Promise<boolean>) | null) => {
  editorSaver = f;
};
/** true when the editor has nothing unsaved or the user chose to save it or drop it; false on cancel or a failed save */
export const confirmDiscard = async () => {
  if (!editorDirty()) return true;
  const r = await askSave("Há alterações por guardar neste ficheiro.", "Editor");
  if (r === "discard") return true;
  if (r === "cancel") return false;
  return (await editorSaver?.()) === true;
};
/** runs `fn` now, or after the user agrees to drop unsaved editor text */
export const guarded = (fn: () => void) => {
  if (!editorDirty()) return fn();
  void confirmDiscard().then((ok) => {
    if (ok) {
      setEditorDirty(false);
      fn();
    }
  });
};

/** bumped whenever the daemon reports file changes in the selected worktree */
export const [fsTick, setFsTick] = createSignal(0);

/** right panel tab */
export const [rightTab, setRightTab] = createSignal<"files" | "changes" | "pulls">("files");

/** What fills the main area. One value, so two views can never be "open" at once. */
export type View =
  | { kind: "terminal" }
  | { kind: "kanban" }
  | { kind: "docker" }
  /** the commit whose patch replaces the terminal area */
  | { kind: "commit"; commit: Commit }
  /** a file open in the editor (relative to the worktree); `line` is where to put the cursor when it was reached from a path link */
  | { kind: "file"; path: string; line: number | null }
  /** which file's diff is showing instead of a terminal */
  | { kind: "diff"; path: string };
const TERMINAL: View = { kind: "terminal" };
export const [view, setView] = createSignal<View>(TERMINAL);

// The readers and setters below are the old per-view signals, now views onto `view`: setting one replaces the whole view,
// and "closing" a view (null / false) only does anything while that view is the one showing.
const kindOf = <K extends View["kind"]>(k: K) => {
  const v = view();
  return v.kind === k ? (v as Extract<View, { kind: K }>) : null;
};
const closeIf = (k: View["kind"]) => setView((v) => (v.kind === k ? TERMINAL : v));
export const openCommit = (): Commit | null => kindOf("commit")?.commit ?? null;
export const setOpenCommit = (c: Commit | null) => (c ? setView({ kind: "commit", commit: c }) : closeIf("commit"));
export const openFile = (): string | null => kindOf("file")?.path ?? null;
export const setOpenFile = (p: string | null) => (p !== null ? setView({ kind: "file", path: p, line: null }) : closeIf("file"));
export const openDiff = (): string | null => kindOf("diff")?.path ?? null;
export const setOpenDiff = (p: string | null) => (p !== null ? setView({ kind: "diff", path: p }) : closeIf("diff"));
/** line to put the cursor on when the open file was reached from a path link; null = top */
export const fileLine = (): number | null => kindOf("file")?.line ?? null;
export const setFileLine = (n: number | null) => setView((v) => (v.kind === "file" ? { ...v, line: n } : v));
/** the project's kanban board replaces the terminal area when true */
export const showKanban = () => view().kind === "kanban";
export const setShowKanban = (on: boolean) => (on ? setView({ kind: "kanban" }) : closeIf("kanban"));
/** the Docker screen replaces the terminal area when true */
export const showDocker = () => view().kind === "docker";
export const setShowDocker = (on: boolean) => (on ? setView({ kind: "docker" }) : closeIf("docker"));

/** One place that picks what fills the main area: no argument = the terminal; the views exclude each other. */
export const showView = (v: { diff?: string; file?: string; commit?: Commit; kanban?: boolean; docker?: boolean } = {}) =>
  setView(
    v.diff !== undefined
      ? { kind: "diff", path: v.diff }
      : v.file !== undefined
        ? { kind: "file", path: v.file, line: null }
        : v.commit
          ? { kind: "commit", commit: v.commit }
          : v.kanban
            ? { kind: "kanban" }
            : v.docker
              ? { kind: "docker" }
              : TERMINAL,
  );
/** opens a file (relative to the worktree) in the editor, at `line` when given; asks first if the editor has unsaved text */
export const openFileAt = (path: string, line?: number) =>
  guarded(() => setView({ kind: "file", path, line: line && line > 0 ? line : null }));

/** a second terminal shown beside the active one (split view); null = no split */
export const [splitSession, setSplitSession] = createSignal<number | null>(null);

/** dev-server ports opened by the selected worktree's terminals */
export const [ports, setPorts] = createSignal<number[]>([]);

/** project whose "new worktree" dialog is open: the sidebar shows it, the command palette can ask for it */
export const [creatingWorktree, setCreatingWorktree] = createSignal<string | null>(null);
/** a worktree the command palette asked to remove; the sidebar confirms and does it */
export const [removeRequest, setRemoveRequest] = createSignal<{ project: string; path: string; name: string } | null>(null);

/** agents may look at the screen and drive the mouse and keyboard (held by the daemon, off at every start) */
export const [computerOn, setComputerOn] = createSignal(false);

/** worktrees pinned to the top of their project (remembered in the browser storage) */
const [pins, setPins] = createPersisted<string[]>("axelstra.pins", isStringArray, []);
export const isPinned = (path: string) => pins().includes(norm(path));
export const togglePin = (path: string) => {
  const n = norm(path);
  setPins((cur) => (cur.includes(n) ? cur.filter((x) => x !== n) : [...cur, n]));
};


/** The new list, but with the old object wherever nothing changed: keyed lists then keep their DOM (tabs keep focus,
 *  icons do not reload) instead of being rebuilt on every poll. `sig` is what counts as a change (default: everything);
 *  pass one that leaves out fields that move on every poll. */
export const keepSame = <T>(prev: T[], next: T[], key: (x: T) => string | number, sig: (x: T) => unknown = (x) => x): T[] => {
  const old = new Map(prev.map((x) => [key(x), x] as const));
  const out = next.map((n) => {
    const o = old.get(key(n));
    return o !== undefined && JSON.stringify(sig(o)) === JSON.stringify(sig(n)) ? o : n;
  });
  // nothing changed at all: the old array itself, so signals and memos downstream do not run for nothing
  return out.length === prev.length && out.every((x, i) => x === prev[i]) ? prev : out;
};

