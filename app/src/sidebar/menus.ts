import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { hasIcon } from "../Icon";
import type { MenuItem } from "../ContextMenu";
import { openInEditor, worktreeSetParent } from "../api";
import { wtKey } from "../wtGroups";
import { P } from "../strings.panels";
import { S } from "../strings";
import { agentLabel } from "../agentLabel";
import {
  canonAgent,
  agentKeys,
  isPinned,
  norm,
  projectPrefsOf,
  projectTerse,
  setAttention,
  setProjectPref,
  togglePin,
} from "../store";
import type { SidebarActions } from "./actions";

const T = P.sidebar.menu;

type Wt = { path: string; branch: string | null; is_main: boolean; parent?: string | null };
type Proj = { id: string; name: string; setup?: string | null; worktrees?: Wt[]; default_branch?: string | null };

export type MenuDeps = Pick<SidebarActions, "run" | "copy" | "sleepWorktree" | "dropWorktree" | "dropProject" | "rebaseWorktree"> & {
  /** reloads what the rebase badge shows */
  afterRebase: () => void;
  /** asks for the worktree-rename dialog */
  rename: (path: string, current: string, label: string) => void;
  newWorktree: (projectId: string) => void;
  editSetup: (projectId: string, setup: string) => void;
  /** types the project setup commands into a new visible terminal of this worktree */
  runSetup: (projectId: string, path: string, setup: string | null | undefined) => void;
  refresh: () => Promise<void>;
};

/** "Mover para…": the other worktrees of the project, minus itself and what is already below it (that would loop) */
const moveMenu = (d: MenuDeps, projectId: string, worktrees: Wt[], w: Wt, below: string[]): MenuItem[] => [
  {
    label: T.moveNone,
    icon: w.parent ? undefined : "check",
    run: () => d.run(() => worktreeSetParent(projectId, w.path, null)),
  },
  ...worktrees
    .filter((x) => !x.is_main && !below.includes(wtKey(x.path)))
    .map((x) => ({
      label: x.branch ?? x.path,
      icon: wtKey(w.parent) === wtKey(x.path) ? "check" : undefined,
      run: () => d.run(() => worktreeSetParent(projectId, w.path, x.path)),
    })),
];

export const worktreeMenu = (d: MenuDeps, p: Proj, w: Wt, label: string, below: string[] = []): MenuItem[] => [
  { label: T.refresh, icon: "refresh", run: d.refresh },
  {
    label: S.replies,
    icon: "sparkle",
    children: [
      {
        label: T.followGlobal,
        icon: projectTerse(p.id) ? undefined : "check",
        run: () => setProjectPref(p.id, { terse: undefined }),
      },
      ...(["off", "short", "tiny"] as const).map((v) => ({
        label: S.terse[v].label,
        icon: projectTerse(p.id) === v ? "check" : undefined,
        run: () => setProjectPref(p.id, { terse: v }),
      })),
    ],
  },
  { separator: true },
  {
    label: T.openIn,
    icon: "folder",
    children: [
      { label: T.explorer, run: () => d.run(() => revealItemInDir(w.path)) },
      { label: "VS Code", run: () => d.run(() => openInEditor(w.path, "code")) },
    ],
  },
  { label: T.copyPath, icon: "copy", run: () => d.copy(w.path) },
  { label: T.runSetup, icon: "play", disabled: !p.setup?.trim(), run: () => d.runSetup(p.id, w.path, p.setup) },
  { label: T.rename, icon: "pencil", disabled: w.is_main, run: () => d.rename(w.path, w.branch ?? label, label) },
  { label: T.copyName, icon: "copy", run: () => d.copy(w.branch ?? label) },
  {
    label: T.moveTo,
    icon: "branch",
    disabled: w.is_main || !p.worktrees || p.worktrees.filter((x) => !x.is_main).length < 2,
    children: p.worktrees ? moveMenu(d, p.id, p.worktrees, w, below) : [],
  },
  { separator: true },
  { label: isPinned(w.path) ? T.unpin : T.pin, icon: "pin", run: () => togglePin(w.path) },
  { label: T.markUnread, icon: "circleHalf", run: () => setAttention((cur) => new Set(cur).add(norm(w.path))) },
  {
    label: P.sidebar.rebase.menu(p.default_branch ?? "main"),
    icon: "branch",
    disabled: w.is_main || !w.branch,
    run: () => d.rebaseWorktree(p.id, w.path, label, p.default_branch ?? "main", d.afterRebase),
  },
  { separator: true },
  { label: T.sleep, icon: "moon", run: () => d.sleepWorktree(w.path, label) },
  {
    label: P.sidebar.removeWorktree,
    icon: "x",
    danger: true,
    disabled: w.is_main,
    run: () => d.dropWorktree(p.id, w.path, label),
  },
];

export const projectMenu = (
  d: MenuDeps,
  p: { id: string; name: string; path: string; setup?: string | null },
): MenuItem[] => [
  { label: P.sidebar.newWorktree, icon: "plus", run: () => d.newWorktree(p.id) },
  { label: T.setupCmd, icon: "gear", run: () => d.editSetup(p.id, p.setup ?? "") },
  {
    label: T.defaultAgent,
    icon: "sparkle",
    children: [
      {
        label: T.followGlobal,
        icon: projectPrefsOf(p.id).agent ? undefined : "check",
        run: () => setProjectPref(p.id, { agent: undefined }),
      },
      ...agentKeys().map((a) => ({
        label:
          projectPrefsOf(p.id).agent && canonAgent(projectPrefsOf(p.id).agent!) === a
            ? `${agentLabel(a)} ✓`
            : agentLabel(a),
        icon: hasIcon(a) ? a : undefined,
        run: () => setProjectPref(p.id, { agent: a }),
      })),
    ],
  },
  { separator: true },
  {
    label: T.openIn,
    icon: "folder",
    children: [{ label: T.explorer, run: () => d.run(() => revealItemInDir(p.path)) }],
  },
  { label: T.copyPath, icon: "copy", run: () => d.copy(p.path) },
  { separator: true },
  { label: P.sidebar.dropLabel, icon: "x", danger: true, run: () => d.dropProject(p.id, p.name) },
];
