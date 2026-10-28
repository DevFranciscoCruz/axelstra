import { JSX, Show } from "solid-js";
import { Icon } from "./Icon";
import { Button, TabsTrigger, cn } from "./ui";

/** One tab of the strip, inside `Tabs` + `TabsList`: a tab trigger, plus a separate close button beside it (a button
 *  never sits inside another). The wrapper takes the drag and mouse handlers; `editing` replaces the label while the
 *  name is being edited. `value` is the tab's key in the strip. */
export function Tab(props: {
  value: string;
  selected: boolean;
  onSelect: () => void;
  onClose?: () => void;
  closeLabel?: string;
  onRename?: () => void;
  /** Alt+Left / Alt+Right on the focused tab: move it one place (the keyboard way to reorder) */
  onMove?: (dir: -1 | 1) => void;
  title?: string;
  id?: string;
  class?: string;
  /** a drop marker on the left edge while another tab is dragged over this one */
  dropBefore?: boolean;
  wrap?: JSX.HTMLAttributes<HTMLDivElement>;
  editing?: JSX.Element;
  children: JSX.Element;
}) {
  return (
    <div
      {...props.wrap}
      class={cn(
        "group flex max-w-[220px] flex-none cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-t-md px-2.5 py-1.5 text-muted-foreground hover:text-foreground data-[active]:bg-background data-[active]:text-foreground",
        props.dropBefore && "shadow-[inset_2px_0_0_var(--accent)]",
        props.class,
      )}
      data-active={props.selected ? "" : undefined}
      role="presentation"
    >
      <Show
        when={props.editing}
        fallback={
          <TabsTrigger
            variant="bare"
            value={props.value}
            selected={props.selected}
            id={props.id}
            data-tab={props.id}
            title={props.title}
            onClick={props.onSelect}
            // middle click closes, like a browser tab (mousedown is stopped so Windows does not start autoscroll)
            onMouseDown={(e: MouseEvent) => e.button === 1 && props.onClose && e.preventDefault()}
            onAuxClick={(e: MouseEvent) => {
              if (e.button !== 1 || !props.onClose) return;
              e.preventDefault();
              props.onClose();
            }}
            onKeyDown={(e: KeyboardEvent) => {
              if (
                e.altKey &&
                !e.ctrlKey &&
                !e.shiftKey &&
                (e.key === "ArrowLeft" || e.key === "ArrowRight") &&
                props.onMove
              ) {
                e.preventDefault();
                e.stopPropagation();
                props.onMove(e.key === "ArrowLeft" ? -1 : 1);
                return;
              }
              if (e.key === "F2" && props.onRename) {
                e.preventDefault();
                props.onRename();
              }
            }}
          >
            {props.children}
          </TabsTrigger>
        }
      >
        {props.editing}
      </Show>
      <Show when={props.onClose}>
        <Button
          variant="icon"
          size="bare"
          class="tab-close invisible rounded-sm p-0.5 text-lg leading-none group-hover:visible group-focus-within:visible group-data-[active]:visible"
          title={props.closeLabel ?? "Fechar"}
          aria-label={props.closeLabel ?? "Fechar"}
          // arrows, Home and End belong to the tabs, not to the close button
          onKeyDown={(e) => ["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key) && e.stopPropagation()}
          onClick={() => props.onClose!()}
        >
          <Icon name="x" />
        </Button>
      </Show>
    </div>
  );
}
