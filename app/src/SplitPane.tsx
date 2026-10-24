import { Show, createSignal, onCleanup } from "solid-js";
import { startDrag } from "./dragResize";
import { Icon } from "./Icon";
import { splitTo } from "./layout";
import { refreshSessions, setSplitSession, splitSession } from "./store";
import { S } from "./strings";
import { TerminalView } from "./TerminalView";
import { Button } from "./ui";

/** The second terminal beside the active one: draggable and keyboard-resizable separator, a close button and the terminal itself. */
export function SplitPane() {
  const [frac, setFrac] = createSignal(0.5);
  let stopDrag: (() => void) | undefined;
  onCleanup(() => stopDrag?.());
  return (
    <>
      <div
        class="split-sep"
        role="separator"
        aria-orientation="vertical"
        aria-label={S.app.splitWidth}
        aria-valuenow={Math.round(frac() * 100)}
        aria-valuemin={20}
        aria-valuemax={80}
        tabindex="0"
        onKeyDown={(e) => {
          const to = splitTo(frac(), e.key);
          if (to === null) return;
          e.preventDefault();
          setFrac(to);
        }}
        onMouseDown={(e) => {
          e.preventDefault();
          const area = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
          stopDrag?.();
          stopDrag = startDrag((m) => setFrac(Math.min(0.8, Math.max(0.2, (area.right - m.clientX) / area.width))));
        }}
      />
      <div class="split-pane group/split" style={{ flex: `0 0 ${frac() * 100}%` }}>
        <Button
          variant="outline"
          size="bare"
          class="absolute right-2 top-1 z-[5] size-5 border-border/60 bg-popover leading-none text-muted-foreground opacity-0 transition-opacity focus-visible:opacity-100 group-hover/split:opacity-100"
          title={S.app.closeSplit}
          aria-label={S.app.closeSplit}
          onClick={() => setSplitSession(null)}
        >
          <Icon name="x" />
        </Button>
        <Show when={splitSession()} keyed>
          {(id) => <TerminalView id={id} focus={false} onExit={() => void refreshSessions()} />}
        </Show>
      </div>
    </>
  );
}
