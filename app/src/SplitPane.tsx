import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { clampSplit, splitBounds, splitTo } from "./layout";
import { createPersisted } from "./persist";
import { startDrag } from "./dragResize";
import { Icon } from "./Icon";
import { exitCodeOf, refreshSessions, sessionGone, setSplitSession, splitSession } from "./store";
import { S } from "./strings";
import { TerminalView } from "./TerminalView";
import { Button } from "./ui";

/** The second terminal beside the active one: draggable and keyboard-resizable separator, a close button and the terminal itself. */
export function SplitPane() {
  // the share of the right pane is remembered between splits; each pane keeps a usable width (layout.ts: MIN_SPLIT_PX)
  const [saved, setSaved] = createPersisted<number>(
    "axelstra.splitFrac",
    (v): v is number => typeof v === "number" && v > 0 && v < 1,
    0.5,
  );
  const [width, setWidth] = createSignal(0);
  const frac = () => clampSplit(saved(), width());
  const setFrac = (f: number) => setSaved(f);
  let stopDrag: (() => void) | undefined;
  onCleanup(() => stopDrag?.());
  // the area is the parent of the separator: its width changes with the window and the side panels
  let sep!: HTMLDivElement;
  onMount(() => {
    const area = sep.parentElement;
    if (!area) return;
    setWidth(area.getBoundingClientRect().width);
    const ro = new ResizeObserver(() => setWidth(area.getBoundingClientRect().width));
    ro.observe(area);
    onCleanup(() => ro.disconnect());
  });
  return (
    <>
      <div
        class="split-sep"
        role="separator"
        aria-orientation="vertical"
        aria-label={S.app.splitWidth}
        aria-valuenow={Math.round(frac() * 100)}
        aria-valuemin={Math.round(splitBounds(width()).min * 100)}
        aria-valuemax={Math.round(splitBounds(width()).max * 100)}
        tabindex="0"
        ref={sep}
        onKeyDown={(e) => {
          const to = splitTo(frac(), e.key, width());
          if (to === null) return;
          e.preventDefault();
          setFrac(to);
        }}
        onMouseDown={(e) => {
          e.preventDefault();
          const area = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
          stopDrag?.();
          stopDrag = startDrag((m) => setFrac(clampSplit((area.right - m.clientX) / area.width, area.width)));
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
          {(id) => (
            <TerminalView
              id={id}
              focus={false}
              onExit={() => void refreshSessions()}
              onGone={() => sessionGone(id)}
              exitCode={() => exitCodeOf(id)}
            />
          )}
        </Show>
      </div>
    </>
  );
}
