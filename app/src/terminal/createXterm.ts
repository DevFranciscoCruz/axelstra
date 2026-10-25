import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { SearchAddon } from "@xterm/addon-search";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { openUrl } from "@tauri-apps/plugin-opener";
import { fsStat, openHtmlFile } from "../api";
import { errorText } from "../appError";
import { K, isMac } from "../keyboard";
import { findPathLinks, resolvePathLink, safeLinkUrl, type PathLink } from "../pathlinks";
import { ask } from "../Confirm";
import { S } from "../strings";
import { P } from "../strings.panels";
import { THEMES, cursorBlinks, termPrefs } from "../termprefs";
import { openFileAt, selectedWorktree, showNotice } from "../store";
import { findHtmlLinks } from "./htmlLinks";

/** a terminal that starts out of sight keeps less history until it is shown; the visible one keeps the full amount */
export const SCROLLBACK = 5000;
export const SCROLLBACK_HIDDEN = 2000;

/** opens a link from terminal output: http(s) only; `confirmFirst` (OSC 8, where the visible text can hide the target) shows the real URL before leaving */
export async function openTermUrl(raw: string, confirmFirst: boolean) {
  const url = safeLinkUrl(raw);
  if (!url) return void showNotice(P.terminal.linkRefused(raw.slice(0, 80)));
  if (
    confirmFirst &&
    !(await ask(P.terminal.linkAsk(url), {
      title: P.terminal.linkTitle,
      kind: "warning",
      okLabel: P.terminal.open,
      cancelLabel: P.terminal.cancel,
    }))
  )
    return;
  void openUrl(url).catch(() => 0);
}

/** opens a clicked path: only inside the selected worktree (in the editor, at its line when the link has one); paths elsewhere are not revealed */
export async function openPathLink(link: PathLink) {
  const wt = selectedWorktree();
  if (!wt) return;
  const r = resolvePathLink(link.path, wt);
  if ("outside" in r) return void showNotice(P.terminal.outside(r.outside));
  try {
    await fsStat(wt, r.rel); // throws when the file does not exist
  } catch {
    showNotice(S.fileMissing(r.rel));
    return;
  }
  openFileAt(r.rel, link.line);
}

/** the modifier that opens a link: Cmd on macOS, Ctrl elsewhere */
const linkModifier = (e: { ctrlKey: boolean; metaKey: boolean }) => (isMac() ? e.metaKey : e.ctrlKey);

/** opens an .html path from terminal output in the system browser; the host resolves and checks it (cwd/worktree/project), never the webview */
export async function openHtmlPath(session: number, path: string) {
  try {
    await openHtmlFile(session, path);
  } catch (e) {
    showNotice(errorText(e));
  }
}

/** the xterm with its addons and link providers, opened in `host` (no renderer: see TerminalView); `session` is the daemon session whose cwd resolves relative .html paths */
export function createXterm(host: HTMLElement, visible: boolean, session?: number) {
  const prefs = termPrefs();
  const term = new Terminal({
    fontFamily: prefs.fontFamily,
    fontSize: prefs.fontSize,
    scrollback: visible ? SCROLLBACK : SCROLLBACK_HIDDEN,
    cursorBlink: cursorBlinks(),
    screenReaderMode: prefs.screenReader,
    allowProposedApi: true,
    linkHandler: { activate: (_e, uri) => void openTermUrl(uri, true), allowNonHttpProtocols: false },
    theme: THEMES[prefs.theme],
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  // the agents measure text with current Unicode widths (symbols, emoji): xterm's default Unicode 6 table wraps
  // those lines differently and the cursor lands on the wrong row
  term.loadAddon(new Unicode11Addon());
  term.unicode.activeVersion = "11";
  const search = new SearchAddon();
  term.loadAddon(search);
  term.loadAddon(new WebLinksAddon((_e, url) => void openTermUrl(url, false)));
  // .html/.htm paths open in the browser, only while the modifier is held (so plain clicks and selections are left alone).
  // xterm asks for links when the pointer moves, so a modifier change replays the last pointer position.
  let held = false;
  let replaying = false;
  let lastMove: MouseEvent | undefined;
  const setHeld = (v: boolean) => {
    if (v === held) return;
    held = v;
    if (!lastMove || !(lastMove.target instanceof Element)) return;
    replaying = true;
    lastMove.target.dispatchEvent(
      new MouseEvent("mousemove", { clientX: lastMove.clientX, clientY: lastMove.clientY, bubbles: true }),
    );
    replaying = false;
  };
  host.addEventListener("mousemove", (e) => {
    if (replaying) return;
    lastMove = e;
    setHeld(linkModifier(e));
  });
  host.addEventListener("mouseleave", () => (lastMove = undefined));
  host.addEventListener("keydown", (e) => setHeld(linkModifier(e)));
  host.addEventListener("keyup", (e) => setHeld(linkModifier(e)));
  // registered before the path provider: with the modifier held an .html path is the browser's, not the editor's
  term.registerLinkProvider({
    provideLinks(y, cb) {
      if (session === undefined || !held) return cb(undefined);
      const line = term.buffer.active.getLine(y - 1)?.translateToString(true) ?? "";
      const out = findHtmlLinks(line).map((l) => ({
        text: line.slice(l.start, l.end),
        range: { start: { x: l.start + 1, y }, end: { x: l.end, y } },
        activate: (e: MouseEvent) => linkModifier(e) && void openHtmlPath(session, l.path),
        hover: () => void (host.title = K(P.terminal.htmlTip)),
        leave: () => void (host.title = ""),
      }));
      cb(out.length ? out : undefined);
    },
  });
  term.registerLinkProvider({
    provideLinks(y, cb) {
      const line = term.buffer.active.getLine(y - 1)?.translateToString(true) ?? "";
      const out = findPathLinks(line)
        .filter((l) => !(held && session !== undefined && /\.html?$/i.test(l.path)))
        .map((l) => ({
          text: l.text,
          range: { start: { x: l.start + 1, y }, end: { x: l.start + l.text.length, y } },
          activate: () => void openPathLink(l),
        }));
      cb(out.length ? out : undefined);
    },
  });
  term.open(host);
  return { term, fit, search };
}
