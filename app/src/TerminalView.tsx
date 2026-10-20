import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { openUrl } from "@tauri-apps/plugin-opener";
import { onCleanup, onMount } from "solid-js";
import { resizeSession, writeSession } from "./api";

/**
 * One xterm bound to one daemon session. Only the visible terminal is mounted:
 * the daemon keeps the history, so re-attaching replays what was missed.
 */
export function TerminalView(props: { id: number; focus?: boolean; onExit?: (code: number | null) => void }) {
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
    term.loadAddon(new WebLinksAddon((_e, url) => void openUrl(url)));
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
    invoke("session_attach", { id: props.id, onData: channel });

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
    term.onResize(({ cols, rows }) => void resizeSession(props.id, cols, rows));
    void resizeSession(props.id, term.cols, term.rows);

    const ro = new ResizeObserver(() => fit.fit());
    ro.observe(host);
    if (props.focus !== false) term.focus();

    onCleanup(() => {
      gone = true;
      unDrop?.();
      host.removeEventListener("contextmenu", onCtx);
      ro.disconnect();
      void invoke("session_detach", { id: props.id });
      term.dispose();
    });
  });

  return <div class="term-host" ref={host} />;
}
