import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { Icon, hasIcon } from "./Icon";
import { PlainButton } from "./ui/plain";

export type MenuItem =
  | { separator: true }
  | {
      label: string;
      icon?: string;
      run?: () => unknown;
      danger?: boolean;
      disabled?: boolean;
      /** the chosen entry of a dropdown (listbox menus only) */
      selected?: boolean;
      /** submenu; opens on hover */
      children?: MenuItem[];
    };

type Open = { x: number; y: number; items: MenuItem[]; minWidth?: number; listbox?: boolean; onClose?: () => void };
const [menu, setMenu] = createSignal<Open | null>(null);
/** a menu or dropdown is open: it is the topmost layer, so Esc and Tab belong to it and not to a dialog underneath */
export const menuOpen = () => menu() !== null || layers() > 0;
/** other topmost layers (an open Select) count as an open menu; `holdLayer` registers one and returns its release */
const [layers, setLayers] = createSignal(0);
export const holdLayer = () => {
  setLayers((n) => n + 1);
  let held = true;
  return () => {
    if (!held) return;
    held = false;
    setLayers((n) => n - 1);
  };
};
/** where focus was when the menu opened; it goes back there when the menu closes from the keyboard */
let opener: HTMLElement | null = null;
const openMenu = (m: Open) => {
  menu()?.onClose?.();
  opener = document.activeElement as HTMLElement | null;
  setMenu(m);
};
const closeMenu = () => {
  const m = menu();
  setMenu(null);
  m?.onClose?.();
};

/** Right-click menu at the pointer. */
export const showMenu = (e: MouseEvent, items: MenuItem[]) => {
  e.preventDefault();
  e.stopPropagation();
  openMenu({ x: e.clientX, y: e.clientY, items });
};

/** Menu at a fixed point (dropdowns open under their button). */
export const showMenuAt = (x: number, y: number, items: MenuItem[], minWidth?: number) => openMenu({ x, y, items, minWidth });

/** Dropdown list under a button (a Select): same menu, announced as a listbox of options; `onClose` runs however it ends. */
export const showListboxAt = (x: number, y: number, items: MenuItem[], minWidth: number, onClose: () => void) => openMenu({ x, y, items, minWidth, listbox: true, onClose });

function Items(props: { items: MenuItem[]; sub?: boolean; flip?: boolean; listbox?: boolean }) {
  const [openSub, setOpenSub] = createSignal<number | null>(null);
  const [flip, setFlip] = createSignal(false);
  return (
    <div class="ctx" role={props.listbox ? "listbox" : "menu"} classList={{ sub: props.sub, flip: props.sub && props.flip }} style={{ "min-width": "inherit" }}>
      <For each={props.items}>
        {(it, i) =>
          "separator" in it ? (
            <div class="ctx-sep" role="separator" />
          ) : (
            <div
              class="ctx-wrap"
              onMouseEnter={(e) => {
                setOpenSub(it.children ? i() : null);
                setFlip(e.currentTarget.getBoundingClientRect().right + 230 > window.innerWidth);
              }}
            >
              <PlainButton
                class="ctx-item"
                role={props.listbox ? "option" : "menuitem"}
                aria-selected={props.listbox ? !!it.selected : undefined}
                aria-haspopup={it.children ? "menu" : undefined}
                classList={{ danger: it.danger, off: it.disabled }}
                disabled={it.disabled}
                onClick={(e) => {
                  e.stopPropagation();
                  if (it.children) return;
                  closeMenu();
                  void it.run?.();
                }}
              >
                <span class="ctx-icon">{hasIcon(it.icon) ? <Icon name={it.icon!} size={13} /> : (it.icon ?? "")}</span>
                <span class="ctx-label">{it.label}</span>
                <Show when={it.children}>
                  <span class="ctx-arrow"><Icon name="chevRight" size={11} /></span>
                </Show>
              </PlainButton>
              <Show when={it.children && openSub() === i()}>
                <Items items={it.children!} sub flip={flip()} />
              </Show>
            </div>
          )
        }
      </For>
    </div>
  );
}

/** Mount once near the app root. */
export function ContextMenu() {
  onMount(() => {
    const close = closeMenu;
    const key = (e: KeyboardEvent) => {
      if (!menu()) return;
      if (e.key === "Escape" || e.key === "Tab") {
        // handled here, so the dialog under the menu keeps its own Esc and Tab for later
        e.preventDefault();
        close();
        opener?.focus?.();
        return;
      }
      const act = document.activeElement as HTMLElement | null;
      const list = act?.closest(".ctx") ?? document.querySelector(".ctx-root > .ctx");
      const btns = [...(list?.querySelectorAll<HTMLElement>(":scope > .ctx-wrap > .ctx-item:not(:disabled)") ?? [])];
      const at = act ? btns.indexOf(act) : -1;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = e.key === "ArrowDown" ? at + 1 : at - 1;
        btns[(n + btns.length) % btns.length]?.focus();
      } else if ((e.key === "ArrowRight" || e.key === "Enter") && act?.getAttribute("aria-haspopup")) {
        e.preventDefault();
        act.closest(".ctx-wrap")?.dispatchEvent(new MouseEvent("mouseenter"));
        queueMicrotask(() => act.closest(".ctx-wrap")?.querySelector<HTMLElement>(".ctx.sub .ctx-item:not(:disabled)")?.focus());
      } else if (e.key === "ArrowLeft") {
        const wrap = act?.closest(".ctx.sub")?.closest(".ctx-wrap");
        if (wrap) {
          e.preventDefault();
          wrap.querySelector<HTMLElement>(":scope > .ctx-item")?.focus();
        }
      }
    };
    window.addEventListener("click", close);
    window.addEventListener("blur", close);
    window.addEventListener("keydown", key);
    window.addEventListener("resize", close);
    onCleanup(() => {
      window.removeEventListener("click", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", key);
      window.removeEventListener("resize", close);
    });
  });
  return (
    <Show when={menu()}>
      {(m) => {
        // keep the menu inside the window
        const left = () => Math.min(m().x, window.innerWidth - 230);
        const top = () => Math.min(m().y, Math.max(8, window.innerHeight - 40 - m().items.length * 30));
        return (
          <div class="ctx-root" ref={(el) => queueMicrotask(() => el.querySelector<HTMLElement>(".ctx-item:not(:disabled)")?.focus())} style={{ left: `${left()}px`, top: `${top()}px`, "min-width": m().minWidth ? `${m().minWidth}px` : undefined }} onContextMenu={(e) => e.preventDefault()}>
            <Items items={m().items} listbox={m().listbox} />
          </div>
        );
      }}
    </Show>
  );
}
