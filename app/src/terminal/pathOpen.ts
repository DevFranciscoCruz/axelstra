import { clipboardWrite, openHtmlFile, pathAction, resolvePath, type PathAction, type PathInfo } from "../api";
import { errorText } from "../appError";
import { askChoice } from "../Confirm";
import { isRemote } from "../paths";
import { resolvePathLink, type PathLink } from "../pathlinks";
import { P } from "../strings.panels";
import { openFileAt, selectedWorktree } from "../store";
import { toast } from "../toasts";

/** what Ctrl+click does with a path: `html` goes to the browser, `text` (code, config, docs) to the editor, anything else asks first */
export type PathClass = "html" | "text" | "other";

const TEXT_EXT = new Set(
  (
    "ts tsx mts cts js jsx mjs cjs json jsonc json5 md mdx txt log rs toml yaml yml css scss sass less xml csv tsv svg " +
    "py rb go java kt kts swift c h cc cpp hpp cs php lua sql graphql gql proto vue svelte astro ini cfg conf env lock " +
    "diff patch gitignore gitattributes editorconfig dockerignore tf hcl dart ex exs erl hs ml r jl zig nim"
  ).split(" "),
);
const TEXT_NAMES = new Set([
  "makefile",
  "dockerfile",
  "license",
  "readme",
  "changelog",
  "authors",
  "notice",
  "procfile",
  "gemfile",
  "rakefile",
  "justfile",
]);

/** the last path segment's extension, lowercase ("" when none; a dotfile like `.gitignore` counts its name as the extension) */
const extOf = (path: string) => {
  const name = path.replace(/\\/g, "/").split("/").pop() ?? "";
  const i = name.lastIndexOf(".");
  return i < 0 ? "" : name.slice(i + 1).toLowerCase();
};
const nameOf = (path: string) => (path.replace(/\\/g, "/").split("/").pop() ?? "").toLowerCase();

/** by name only; whether the path exists and what it really is the daemon says */
export const classifyPath = (path: string): PathClass => {
  const ext = extOf(path);
  if (ext === "html" || ext === "htm") return "html";
  if (TEXT_EXT.has(ext) || (ext === "" && TEXT_NAMES.has(nameOf(path)))) return "text";
  return "other";
};

/** the buttons the dialog offers for a path, in order: what the daemon allows, nothing else */
export const choicesFor = (info: Pick<PathInfo, "can_open" | "can_run">): { id: PathAction; label: string }[] => [
  ...(info.can_run ? [{ id: "run" as const, label: P.terminal.pathRun }] : []),
  ...(info.can_open ? [{ id: "open" as const, label: P.terminal.pathOpen }] : []),
  { id: "reveal" as const, label: P.terminal.pathReveal },
];

/** the dialog text for a path: the canonical path, its type and why some buttons are missing */
export const describe = (info: PathInfo): string => {
  const kind = info.is_dir ? P.terminal.kindFolder : info.exec ? P.terminal.kindProgram : P.terminal.kindFile;
  const note = info.can_run ? P.terminal.pathRunNote : !info.inside ? P.terminal.pathOutsideNote : "";
  return P.terminal.pathMessage(info.path, kind, note);
};

export type PathDeps = {
  worktree: () => string | undefined;
  resolve: typeof resolvePath;
  act: typeof pathAction;
  openHtml: typeof openHtmlFile;
  openEditor: (rel: string, line?: number) => void;
  choose: typeof askChoice;
  copy: (text: string) => Promise<unknown>;
  warn: (text: string) => void;
};

const real: PathDeps = {
  worktree: () => selectedWorktree() ?? undefined,
  resolve: (...a) => resolvePath(...a),
  act: (...a) => pathAction(...a),
  openHtml: (...a) => openHtmlFile(...a),
  openEditor: (rel, line) => openFileAt(rel, line),
  choose: (...a) => askChoice(...a),
  copy: (t) => clipboardWrite(t),
  // a warning stays until closed, so a click is never answered with silence
  warn: (text) => void toast({ text, kind: "warn", key: `path:${text}` }),
};

/**
 * Ctrl+click on a path from terminal output. It always answers: the editor (code and text inside the worktree), the browser
 * (.html), a warning (path missing) or a dialog (everything else: programs, scripts, folders, binaries, anything outside the
 * project). Nothing is run, opened or revealed except from a button of that dialog; the daemon revalidates each action.
 * Must only be called from a real Ctrl+click of the user (the caller checks `isTrusted`).
 */
export async function openPath(link: PathLink, session: number | undefined, d: PathDeps = real) {
  const wt = d.worktree();
  if (session === undefined || !wt) return d.warn(P.terminal.pathNotFound(link.path));
  if (isRemote(wt)) {
    // the file is on another machine: nothing here can reveal or run it
    const c = await d.choose(P.terminal.pathMessage(link.path, P.terminal.kindFile, P.terminal.pathRemoteNote), {
      title: P.terminal.pathTitle,
      cancelLabel: P.terminal.cancel,
      choices: [{ id: "copy", label: P.terminal.pathCopy }],
    });
    if (c === "copy") await d.copy(link.path).catch(() => 0);
    return;
  }
  let info: PathInfo;
  try {
    info = await d.resolve(session, link.path);
  } catch (e) {
    return d.warn(errorText(e));
  }
  const kind = classifyPath(link.path);
  if (kind === "html" && !info.is_dir) {
    try {
      await d.openHtml(session, link.path);
    } catch (e) {
      d.warn(errorText(e));
    }
    return;
  }
  if (kind === "text" && !info.is_dir) {
    const r = resolvePathLink(link.path, wt);
    if ("rel" in r) return d.openEditor(r.rel, link.line);
  }
  const choices = choicesFor(info);
  const id = await d.choose(describe(info), { title: P.terminal.pathTitle, cancelLabel: P.terminal.cancel, choices });
  // the answer must be one of the buttons that were offered
  const action = choices.find((c) => c.id === id)?.id;
  if (!action) return;
  try {
    await d.act(session, info.path, action);
  } catch (e) {
    d.warn(errorText(e));
  }
}
