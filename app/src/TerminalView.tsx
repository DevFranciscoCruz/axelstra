import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { WebglAddon } from "@xterm/addon-webgl";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { Icon } from "./Icon";
import { fsStat, rememberTermSize, resizeSession, writeSession } from "./api";
import { findPathLinks, resolvePathLink, type PathLink } from "./pathlinks";
import { S } from "./strings";
import { attachWithRetry, createSizeSync, createWriteBatcher } from "./termsize";
import { THEMES, setTermPrefs, termPrefs, zoomFont } from "./termprefs";
import { openFileAt, selectedWorktree, showNotice } from "./store";
import "./termui.css";

/** a terminal that starts out of sight keeps less history until it is shown; the visible one keeps the full amount */
const SCROLLBACK = 5000;
const SCROLLBACK_HIDDEN = 2000;

/** opens a clicked path: inside the selected worktree in the editor (at its line when the link has one), elsewhere in the file manager */
async function openPathLink(link: PathLink) {
  const wt = selectedWorktree();
  if (!wt) return;
  const r = resolvePathLink(link.path, wt);
  if ("outside" in r) return void revealItemInDir(r.outside).catch(() => showNotice(S.fileMissing(r.outside)));
  try {
    await fsStat(wt, r.rel); // throws when the file does not exist
  } catch {
    showNotice(S.fileMissing(r.rel));
    return;
  }
  openFileAt(r.rel, link.line);
}

/**
 * One xterm bound to one daemon session. Only the visible terminal is mounted:
 * the daemon keeps the history, so re-attaching replays what was missed.
 */
export function TerminalView(props: { id: number; focus?: boolean; active?: () => boolean; onExit?: (code: number | null) => void }) {
  let host!: HTMLDivElement;
  let findInput: HTMLInputElement | undefined;
  const [finding, setFinding] = createSignal(false);

  onMount(() => {
    const isActive = () => props.active?.() ?? true;
    const prefs = termPrefs();
    const term = new Terminal({
      fontFamily: prefs.fontFamily,
      fontSize: prefs.fontSize,
      scrollback: isActive() ? SCROLLBACK : SCROLLBACK_HIDDEN,
      cursorBlink: true,
      allowProposedApi: true,
      theme: THEMES[prefs.theme],
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    const search = new SearchAddon();
    term.loadAddon(search);
    term.loadAddon(new WebLinksAddon((_e, url) => void openUrl(url)));
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
    // The GPU renderer costs memory per terminal and the browser allows only a few contexts: only the visible terminal has one.
    let gl: WebglAddon | null = null;
    const loadGl = () => {
      if (gl) return;
      try {
        const g = new WebglAddon();
        g.onContextLoss(() => {
          g.dispose();
          if (gl === g) gl = null;
        });
        term.loadAddon(g);
        gl = g;
      } catch {
        /* canvas renderer fallback */
      }
    };
    const dropGl = () => {
      gl?.dispose();
      gl = null;
    };
    if (isActive()) loadGl();
    fit.fit();

    // output arrives in many small chunks: one write per frame (a hidden terminal can wait longer)
    const batch = createWriteBatcher((d) => term.write(d), { delay: () => (isActive() ? 8 : 120) });
    const channel = new Channel<ArrayBuffer | { event: string; code: number | null }>();
    channel.onmessage = (m) => {
      if (m instanceof ArrayBuffer) batch.push(new Uint8Array(m));
      else if (m && (m as { event?: string }).event === "session.exit") {
        batch.flush();
        props.onExit?.((m as { code: number | null }).code);
      }
    };
    // a per-attach token (when the backend hands one out) tells session_detach which attach to end
    let token: unknown;
    const detach = () => void invoke("session_detach", { id: props.id, token: token ?? undefined }).catch(() => 0);
    void attachWithRetry(() => invoke<unknown>("session_attach", { id: props.id, onData: channel }), 600, {
      gone: () => gone,
      onFail: (e) => term.write(`\r\n\x1b[31m[axelstra] ${S.attachFailed}${String(e)}\x1b[0m\r\n`),
    }).then((r) => {
      // only a terminal that really attached tells the PTY its size
      if (!r.ok) return;
      token = r.value ?? undefined;
      if (gone) {
        if (token !== undefined) detach();
        return;
      }
      sizeSync.forget();
      pushSize();
    });

    // WebView2 gates navigator.clipboard behind a permission prompt, so the clipboard
    // goes through the native side. Images are saved to a temp PNG and typed as a path.
    const quote = (p: string) => (/\s/.test(p) ? `"${p}"` : p);
    const pasteFromClipboard = async () => {
      try {
        const c = await invoke<{ kind: string; value: string }>("clipboard_read");
        if (c.kind === "image") term.paste(quote(c.value) + " ");
        else if (c.kind === "text") term.paste(c.value);
      } catch {
        /* clipboard unavailable */
      }
    };
    const copySelection = () => void invoke("clipboard_write", { text: term.getSelection() }).catch(() => 0);

    const openFind = () => {
      setFinding(true);
      const sel = term.getSelection();
      queueMicrotask(() => {
        if (sel && findInput) findInput.value = sel.split("\n")[0];
        findInput?.focus();
        findInput?.select();
      });
    };
    const closeFind = () => {
      setFinding(false);
      search.clearDecorations();
      term.focus();
    };
    const runFind = (back: boolean, incremental = false) => {
      const q = findInput?.value ?? "";
      if (!q) return void search.clearDecorations();
      if (back) search.findPrevious(q);
      else search.findNext(q, { incremental });
    };

    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      const k = e.key.toLowerCase();
      if (e.ctrlKey && !e.altKey && k === "v") {
        e.preventDefault();
        void pasteFromClipboard();
        return false;
      }
      if (e.ctrlKey && !e.altKey && k === "c" && term.hasSelection()) {
        copySelection();
        term.clearSelection();
        return false;
      }
      if (e.ctrlKey && !e.altKey && !e.shiftKey && k === "f") {
        e.preventDefault();
        openFind();
        return false;
      }
      // font zoom: Ctrl+= / Ctrl++ bigger, Ctrl+- smaller, Ctrl+0 back to the default; kept in the preferences
      if (e.ctrlKey && !e.altKey) {
        const delta = k === "=" || k === "+" ? 1 : k === "-" || k === "_" ? -1 : k === "0" ? 0 : null;
        if (delta !== null) {
          e.preventDefault();
          setTermPrefs({ fontSize: zoomFont(term.options.fontSize ?? termPrefs().fontSize, delta) });
          return false;
        }
      }
      return true;
    });
    // right click: copy a selection, otherwise paste (Windows Terminal behaviour)
    const onCtx = (e: MouseEvent) => {
      e.preventDefault();
      if (term.hasSelection()) {
        copySelection();
        term.clearSelection();
      } else void pasteFromClipboard();
    };
    host.addEventListener("contextmenu", onCtx);

    // files dropped from the OS arrive as native events carrying paths
    let unDrop: (() => void) | undefined;
    let gone = false;
    void getCurrentWebview()
      .onDragDropEvent((ev) => {
        if (ev.payload.type !== "drop") return;
        const r = host.getBoundingClientRect();
        const x = ev.payload.position.x / devicePixelRatio;
        const y = ev.payload.position.y / devicePixelRatio;
        if (x < r.left || x > r.right || y < r.top || y > r.bottom) return;
        term.paste(ev.payload.paths.map(quote).join(" ") + " ");
        term.focus();
      })
      .then((u) => (gone ? u() : (unDrop = u)));

    term.onData((d) => void writeSession(props.id, d));
    // see termsize.ts: the PTY must always end up with the size xterm shows
    const visible = () => host.clientWidth > 0 && host.clientHeight > 0;
    const sizeSync = createSizeSync({ size: () => term, visible, gone: () => gone, send: (c, r) => resizeSession(props.id, c, r) });
    const pushSize = sizeSync.push;
    term.onResize(sizeSync.resized);

    // a hidden terminal (display: none) has no size: fitting it would shrink its PTY to nothing
    const refit = () => {
      if (!visible()) return;
      fit.fit();
      rememberTermSize(term.cols, term.rows);
      pushSize();
    };
    refit();
    const ro = new ResizeObserver(refit);
    ro.observe(host);

    // look and font from Settings (and the zoom keys): applied live, then the grid is fitted again
    createEffect(() => {
      const p = termPrefs();
      term.options.theme = THEMES[p.theme];
      term.options.fontFamily = p.fontFamily;
      term.options.fontSize = p.fontSize;
      requestAnimationFrame(() => {
        try {
          term.clearTextureAtlas();
        } catch {
          /* canvas renderer has no atlas */
        }
        refit();
      });
    });

    // Moving the window to a screen with another scale leaves the WebGL glyph atlas at the old size (garbled,
    // overlapping text): rebuild it and refit whenever the pixel ratio changes.
    let dpr: MediaQueryList | null = null;
    const onDprChange = () => {
      try {
        term.clearTextureAtlas();
      } catch {
        /* canvas renderer has no atlas */
      }
      refit();
      term.refresh(0, term.rows - 1);
      watchDpr();
    };
    const watchDpr = () => {
      dpr?.removeEventListener("change", onDprChange);
      dpr = matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      dpr.addEventListener("change", onDprChange, { once: true });
    };
    watchDpr();
    if (props.focus !== false && isActive()) term.focus();
    // a terminal that was hidden (another tab in front) is fitted and focused again when it comes back;
    // while hidden it gives up its GPU context
    createEffect(() => {
      if (!isActive()) {
        dropGl();
        return;
      }
      if (term.options.scrollback !== SCROLLBACK) term.options.scrollback = SCROLLBACK;
      requestAnimationFrame(() => {
        if (gone) return;
        loadGl();
        refit();
        term.refresh(0, term.rows - 1);
        if (props.focus !== false) term.focus();
      });
    });

    onCleanup(() => {
      gone = true;
      batch.cancel();
      sizeSync.cancel();
      unDrop?.();
      host.removeEventListener("contextmenu", onCtx);
      ro.disconnect();
      dpr?.removeEventListener("change", onDprChange);
      detach();
      term.dispose();
    });

    // the find bar's handlers need the addon: kept on the host element for the JSX below
    findHandlers = { run: runFind, close: closeFind };
  });

  let findHandlers: { run: (back: boolean, incremental?: boolean) => void; close: () => void } | undefined;

  return (
    <div class="term-host" ref={host}>
      <Show when={finding()}>
        <div class="term-find" role="search" on:contextmenu={(e) => e.stopPropagation()}>
          <input
            ref={findInput}
            type="text"
            placeholder={S.findPlaceholder}
            aria-label={S.findPlaceholder}
            spellcheck={false}
            onInput={() => findHandlers?.run(false, true)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                findHandlers?.run(e.shiftKey);
              } else if (e.key === "Escape") {
                e.preventDefault();
                findHandlers?.close();
              }
            }}
          />
          <button title={S.findPrev} onClick={() => findHandlers?.run(true)}>
            <span class="flip">
              <Icon name="chevDown" />
            </span>
          </button>
          <button title={S.findNext} onClick={() => findHandlers?.run(false)}>
            <Icon name="chevDown" />
          </button>
          <button title={S.findClose} onClick={() => findHandlers?.close()}>
            <Icon name="x" />
          </button>
        </div>
      </Show>
    </div>
  );
}
