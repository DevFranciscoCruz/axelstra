import { Channel, invoke } from "@tauri-apps/api/core";
import { Button, Input } from "./ui";
import { Show, createEffect, onCleanup, onMount, untrack } from "solid-js";
import { Icon } from "./Icon";
import { rememberTermSize, resizeSession, writeSession } from "./api";
import { S } from "./strings";
import { attachWithRetry, createSizeSync, createWriteBatcher } from "./termsize";
import { THEMES, setTermPrefs, termPrefs, zoomFont } from "./termprefs";
import { SCROLLBACK, createXterm } from "./terminal/createXterm";
import { dropInside, onTermDrop } from "./terminal/dragDrop";
import { quotePath, useClipboard } from "./terminal/useClipboard";
import { useFind } from "./terminal/useFind";
import { useWebgl } from "./terminal/useWebgl";
import "./termui.css";

/**
 * One xterm bound to one daemon session. Only the visible terminal is mounted:
 * the daemon keeps the history, so re-attaching replays what was missed.
 */
export function TerminalView(props: { id: number; focus?: boolean; active?: () => boolean; onExit?: (code: number | null) => void }) {
  let host!: HTMLDivElement;
  const find = useFind();

  onMount(() => {
    const isActive = () => props.active?.() ?? true;
    const { term, fit, search } = createXterm(host, isActive());
    find.bind(term, search);
    let gone = false;

    // see termsize.ts: the PTY must always end up with the size xterm shows
    const visible = () => host.clientWidth > 0 && host.clientHeight > 0;
    const sizeSync = createSizeSync({ size: () => term, visible, gone: () => gone, send: (c, r) => resizeSession(props.id, c, r) });
    const pushSize = sizeSync.push;
    // a hidden terminal (display: none) has no size: fitting it would shrink its PTY to nothing
    const refit = () => {
      if (!visible()) return;
      fit.fit();
      rememberTermSize(term.cols, term.rows);
      pushSize();
    };

    const gl = useWebgl(term, () =>
      // the fallback renderer measures cells a little differently: fit the grid again and repaint
      requestAnimationFrame(() => {
        if (gone) return;
        refit();
        term.refresh(0, term.rows - 1);
      }),
    );
    if (isActive()) gl.load();
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

    const clip = useClipboard(term);
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      const k = e.key.toLowerCase();
      if (e.ctrlKey && !e.altKey && k === "v") {
        e.preventDefault();
        void clip.paste();
        return false;
      }
      if (e.ctrlKey && !e.altKey && k === "c" && term.hasSelection()) {
        clip.copy();
        term.clearSelection();
        return false;
      }
      if (e.ctrlKey && !e.altKey && !e.shiftKey && k === "f") {
        e.preventDefault();
        find.open();
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
    host.addEventListener("contextmenu", clip.contextMenu);

    // files dropped from the OS: one shared webview listener for all terminals, each takes the drops over itself
    const unDrop = onTermDrop((p) => {
      if (!dropInside(host, p.position)) return;
      term.paste(p.paths.map(quotePath).join(" ") + " ");
      term.focus();
    });

    term.onData((d) => void writeSession(untrack(() => props.id), d));
    term.onResize(sizeSync.resized);
    refit();
    const ro = new ResizeObserver(refit);
    ro.observe(host);

    // look and font from Settings (and the zoom keys): applied live, then the grid is fitted again
    createEffect(() => {
      const p = termPrefs();
      term.options.theme = THEMES[p.theme];
      term.options.fontFamily = p.fontFamily;
      term.options.fontSize = p.fontSize;
      term.options.screenReaderMode = p.screenReader;
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
        gl.drop();
        return;
      }
      if (term.options.scrollback !== SCROLLBACK) term.options.scrollback = SCROLLBACK;
      requestAnimationFrame(() => {
        if (gone) return;
        gl.load();
        refit();
        term.refresh(0, term.rows - 1);
        if (props.focus !== false) term.focus();
      });
    });

    onCleanup(() => {
      gone = true;
      batch.cancel();
      sizeSync.cancel();
      unDrop();
      host.removeEventListener("contextmenu", clip.contextMenu);
      ro.disconnect();
      dpr?.removeEventListener("change", onDprChange);
      detach();
      term.dispose();
    });
  });

  return (
    <div class="term-host" ref={host}>
      <Show when={find.finding()}>
        <div class="term-find" role="search" on:contextmenu={(e) => e.stopPropagation()}>
          <Input
            ref={find.setInput}
            type="text"
            placeholder={S.findPlaceholder}
            aria-label={S.findPlaceholder}
            spellcheck={false}
            onInput={() => find.run(false, true)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                find.run(e.shiftKey);
              } else if (e.key === "Escape") {
                e.preventDefault();
                find.close();
              }
            }}
          />
          <Button variant="ghost" size="icon" title={S.findPrev} aria-label={S.findPrev} onClick={() => find.run(true)}>
            <span class="flip">
              <Icon name="chevDown" />
            </span>
          </Button>
          <Button variant="ghost" size="icon" title={S.findNext} aria-label={S.findNext} onClick={() => find.run(false)}>
            <Icon name="chevDown" />
          </Button>
          <Button variant="ghost" size="icon" title={S.findClose} aria-label={S.findClose} onClick={() => find.close()}>
            <Icon name="x" />
          </Button>
        </div>
      </Show>
    </div>
  );
}
