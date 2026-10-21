import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { createEffect, onCleanup, onMount } from "solid-js";
import { fsStat, resizeSession, writeSession } from "./api";
import { guarded, norm, selectedWorktree, setOpenCommit, setOpenDiff, setOpenFile, setShowKanban } from "./store";

/** file paths in terminal output: absolute Windows paths or relative ones with an extension, optionally followed by :line[:col] */
const PATH_RE = /(?:[A-Za-z]:[\\/][^\s"'`<>|*?()]+|(?:\.{1,2}[\\/])?[\w@.-]+(?:[\\/][\w@.-]+)*\.[A-Za-z][\w]{0,7})(?::\d+(?::\d+)?)?/g;

/** opens a clicked path: inside the selected worktree in the editor, elsewhere in the file manager */
async function openPathLink(raw: string) {
  const path = raw.replace(/:\d+(?::\d+)?$/, "").replace(/[.,;:]+$/, "");
  const wt = selectedWorktree();
  if (!wt) return;
  const slashed = path.replace(/\\/g, "/");
  const abs = /^[A-Za-z]:\//.test(slashed);
  let rel = slashed.replace(/^\.\//, "");
  if (abs) {
    if (!norm(slashed).startsWith(norm(wt) + "/")) return void revealItemInDir(path).catch(() => 0);
    rel = slashed.slice(wt.replace(/\\/g, "/").replace(/\/$/, "").length + 1);
  }
  try {
    await fsStat(wt, rel); // throws when the file does not exist
  } catch {
    return;
  }
  guarded(() => {
    setOpenDiff(null);
    setShowKanban(false);
    setOpenCommit(null);
    setOpenFile(rel);
  });
}

/**
 * One xterm bound to one daemon session. Only the visible terminal is mounted:
 * the daemon keeps the history, so re-attaching replays what was missed.
 */
export function TerminalView(props: { id: number; focus?: boolean; active?: () => boolean; onExit?: (code: number | null) => void }) {
  let host!: HTMLDivElement;

  onMount(() => {
    const term = new Terminal({
      fontFamily: '"Cascadia Mono", Consolas, monospace',
      fontSize: 13,
      scrollback: 5000,
      cursorBlink: true,
      allowProposedApi: true,
      theme: { background: "#0e0f13", foreground: "#d7dae0", cursor: "#a78bfa" },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    // emoji and wide glyphs agents print take two cells, as in the agent's own width table
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
    term.loadAddon(new WebLinksAddon((_e, url) => void openUrl(url)));
    term.registerLinkProvider({
      provideLinks(y, cb) {
        const line = term.buffer.active.getLine(y - 1)?.translateToString(true) ?? "";
        const out: { range: { start: { x: number; y: number }; end: { x: number; y: number } }; text: string; activate: () => void }[] = [];
        for (const m of line.matchAll(PATH_RE)) {
          const at = m.index ?? 0;
          // a match that starts mid-token (inside a URL, say) is not a path
          if (at > 0 && /[\w:/@.-]/.test(line[at - 1])) continue;
          const text = m[0].replace(/[.,;:)]+$/, "");
          out.push({ text, range: { start: { x: at + 1, y }, end: { x: at + text.length, y } }, activate: () => void openPathLink(text) });
        }
        cb(out.length ? out : undefined);
      },
    });
    term.open(host);
    try {
      const gl = new WebglAddon();
      gl.onContextLoss(() => gl.dispose());
      term.loadAddon(gl);
    } catch {
      /* canvas renderer fallback */
    }
    fit.fit();

    const channel = new Channel<ArrayBuffer | { event: string; code: number | null }>();
    channel.onmessage = (m) => {
      if (m instanceof ArrayBuffer) term.write(new Uint8Array(m));
      else if (m && (m as { event?: string }).event === "session.exit") props.onExit?.((m as { code: number | null }).code);
    };
    const attach = (retry: boolean) =>
      invoke("session_attach", { id: props.id, onData: channel }).catch((e) => {
        if (retry) setTimeout(() => void attach(false), 600);
        else term.write(`\r\n\x1b[31m[axelstra] não foi possível ligar a esta sessão: ${String(e)}\x1b[0m\r\n`);
      });
    void attach(true);

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
    // dragging a panel fires many sizes; only the last one is sent (each resize makes the agent redraw everything)
    let resizeTimer: ReturnType<typeof setTimeout> | undefined;
    term.onResize(({ cols, rows }) => {
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => void resizeSession(props.id, cols, rows), 80);
    });
    void resizeSession(props.id, term.cols, term.rows);

    // a hidden terminal (display: none) has no size: fitting it would shrink its PTY to nothing
    const refit = () => {
      if (host.clientWidth > 0 && host.clientHeight > 0) fit.fit();
    };
    const ro = new ResizeObserver(refit);
    ro.observe(host);

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
    if (props.focus !== false && (props.active?.() ?? true)) term.focus();
    // a terminal that was hidden (another tab in front) is fitted and focused again when it comes back
    createEffect(() => {
      if (!(props.active?.() ?? true)) return;
      requestAnimationFrame(() => {
        refit();
        term.refresh(0, term.rows - 1);
        if (props.focus !== false) term.focus();
      });
    });

    onCleanup(() => {
      gone = true;
      unDrop?.();
      host.removeEventListener("contextmenu", onCtx);
      ro.disconnect();
      dpr?.removeEventListener("change", onDprChange);
      void invoke("session_detach", { id: props.id });
      term.dispose();
    });
  });

  return <div class="term-host" ref={host} />;
}
