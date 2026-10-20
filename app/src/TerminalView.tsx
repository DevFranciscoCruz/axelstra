import { Channel, invoke } from "@tauri-apps/api/core";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { onCleanup, onMount } from "solid-js";
import { resizeSession, writeSession } from "./api";

/**
 * One xterm bound to one daemon session. Only the visible terminal is mounted:
 * the daemon keeps the history, so re-attaching replays what was missed.
 */
export function TerminalView(props: { id: number; onExit?: (code: number | null) => void }) {
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

    term.onData((d) => void writeSession(props.id, d));
    term.onResize(({ cols, rows }) => void resizeSession(props.id, cols, rows));
    void resizeSession(props.id, term.cols, term.rows);

    const ro = new ResizeObserver(() => fit.fit());
    ro.observe(host);
    term.focus();

    onCleanup(() => {
      ro.disconnect();
      void invoke("session_detach", { id: props.id });
      term.dispose();
    });
  });

  return <div class="term-host" ref={host} />;
}
