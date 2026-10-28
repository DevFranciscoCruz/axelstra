import { DropdownMenu as K } from "@kobalte/core/dropdown-menu";
import { For, JSX, Show } from "solid-js";
import { Icon, hasIcon } from "../Icon";
import { cn } from "./cn";

export type DropdownItem =
  | { separator: true }
  | {
      label: string;
      icon?: string;
      run?: () => unknown;
      danger?: boolean;
      disabled?: boolean;
      /** the chosen entry of a dropdown (listbox menus only) */
      selected?: boolean;
      /** submenu; opens on hover or ArrowRight */
      children?: DropdownItem[];
    };

const content =
  "z-[70] min-w-[13rem] rounded-xl border border-border bg-popover p-1 text-popover-foreground shadow-lg outline-hidden";
const row =
  "flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-base text-foreground outline-hidden data-[highlighted]:bg-accent data-[expanded]:bg-accent data-[highlighted]:ring-2 data-[highlighted]:ring-inset data-[highlighted]:ring-ring data-[expanded]:ring-2 data-[expanded]:ring-inset data-[expanded]:ring-ring data-[disabled]:cursor-default data-[disabled]:opacity-40";

const openFirst = (e: Event) => {
  // Kobalte would focus the menu itself; the first enabled entry gets focus instead
  e.preventDefault();
  const list = document.querySelector<HTMLElement>("[data-ax-menu]");
  setTimeout(() => list?.querySelector<HTMLElement>(':scope > [role="menuitem"]:not([data-disabled]), :scope > [role="option"]:not([data-disabled])')?.focus());
};

/** Menu opened at a point (right-click, or under a button), on Kobalte's DropdownMenu with a virtual anchor.
 *  Controlled: the caller owns `open`. Submenus open on hover and ArrowRight, close on ArrowLeft; arrows move,
 *  Enter/Space pick. With `listbox` the entries are announced as options of a listbox instead of menu items.
 *  Esc and Tab are handled by the caller (ContextMenu) so the layer rules of `menuOpen` hold. */
export function DropdownMenu(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  x: number;
  y: number;
  items: DropdownItem[];
  minWidth?: number;
  listbox?: boolean;
  /** an enabled entry without a submenu was chosen */
  onPick: (item: Exclude<DropdownItem, { separator: true }>) => void;
}): JSX.Element {
  const itemRole = () => (props.listbox ? "option" : "menuitem");
  const entries = (items: DropdownItem[]): JSX.Element => (
    <For each={items}>
      {(it) => {
        if ("separator" in it) return <K.Separator class="mx-1 my-1.5 h-px border-0 bg-border" />;
        const body = (
          <>
            <span class={cn("inline-flex w-4 flex-none items-center justify-center text-muted-foreground", it.danger && "text-destructive")}>
              {hasIcon(it.icon) ? <Icon name={it.icon!} size={13} /> : (it.icon ?? "")}
            </span>
            <span class="flex-1">{it.label}</span>
          </>
        );
        if (!it.children) {
          return (
            <K.Item
              class={cn(row, it.danger && "text-destructive")}
              role={itemRole()}
              aria-selected={props.listbox ? !!it.selected : undefined}
              disabled={it.disabled}
              onSelect={() => props.onPick(it)}
            >
              {body}
            </K.Item>
          );
        }
        return (
          <K.Sub gutter={0} shift={-6}>
            <K.SubTrigger class={cn(row, it.danger && "text-destructive")} aria-haspopup="menu" disabled={it.disabled}>
              {body}
              <span class="text-muted-foreground">
                <Icon name="chevRight" size={11} />
              </span>
            </K.SubTrigger>
            <K.Portal>
              <K.SubContent class={content} data-ax-menu onContextMenu={(e: MouseEvent) => e.preventDefault()}>
                {entries(it.children!)}
              </K.SubContent>
            </K.Portal>
          </K.Sub>
        );
      }}
    </For>
  );
  return (
    <K
      open={props.open}
      onOpenChange={props.onOpenChange}
      modal={false}
      preventScroll={false}
      placement="bottom-start"
      gutter={0}
      getAnchorRect={() => ({ x: props.x, y: props.y, width: 0, height: 0 })}
    >
      <Show when={props.open}>
        <K.Portal>
          <K.Content
            class={content}
            role={props.listbox ? "listbox" : "menu"}
            data-ax-menu
            style={{ "min-width": props.minWidth ? `${props.minWidth}px` : undefined }}
            onOpenAutoFocus={openFirst}
            onCloseAutoFocus={(e: Event) => e.preventDefault()}
            onContextMenu={(e: MouseEvent) => e.preventDefault()}
          >
            {entries(props.items)}
          </K.Content>
        </K.Portal>
      </Show>
    </K>
  );
}
