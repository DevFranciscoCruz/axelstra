import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { SearchAddon } from "@xterm/addon-search";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { openUrl } from "@tauri-apps/plugin-opener";
import { fsStat } from "../api";
import { findPathLinks, resolvePathLink, safeLinkUrl, type PathLink } from "../pathlinks";
import { ask } from "../Confirm";
import { S } from "../strings";
import { P } from "../strings.panels";
import { THEMES, cursorBlinks, termPrefs } from "../termprefs";
import { openFileAt, selectedWorktree, showNotice } from "../store";

/** a terminal that starts out of sight keeps less history until it is shown; the visible one keeps the full amount */
export const SCROLLBACK = 5000;
export const SCROLLBACK_HIDDEN = 2000;

/** opens a link from terminal output: http(s) only; `confirmFirst` (OSC 8, where the visible text can hide the target) shows the real URL before leaving */
export async function openTermUrl(raw: string, confirmFirst: boolean) {
  const url = safeLinkUrl(raw);
  if (!url) return void showNotice(P.terminal.linkRefused(raw.slice(0, 80)));
  if (confirmFirst && !(await ask(P.terminal.linkAsk(url), { title: P.terminal.linkTitle, kind: "warning", okLabel: P.terminal.open, cancelLabel: P.terminal.cancel }))) return;
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

/** the xterm with its addons and link providers, opened in `host` (no renderer, no session: see TerminalView) */
export function createXterm(host: HTMLElement, visible: boolean) {
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
  term.registerLinkProvider({
    provideLinks(y, cb) {
      const line = term.buffer.active.getLine(y - 1)?.translateToString(true) ?? "";
      const out = findPathLinks(line).map((l) => ({
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
