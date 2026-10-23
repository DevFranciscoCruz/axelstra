import { createSignal, onCleanup, onMount } from "solid-js";
import { DropdownMenu, type DropdownItem } from "./ui/dropdown-menu";

export type MenuItem = DropdownItem;

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

/** Mount once near the app root. Esc and Tab are taken here (window level) so the dialog under the menu keeps its own. */
export function ContextMenu() {
  onMount(() => {
    const close = closeMenu;
    const key = (e: KeyboardEvent) => {
      if (!menu()) return;
      if (e.key === "Escape" || e.key === "Tab") {
        e.preventDefault();
        close();
        opener?.focus?.();
      }
    };
    window.addEventListener("blur", close);
    window.addEventListener("keydown", key);
    window.addEventListener("resize", close);
    onCleanup(() => {
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", key);
      window.removeEventListener("resize", close);
    });
  });
  // the menu stays mounted; its data is read from the signal only while one is open
  return (
    <DropdownMenu
      open={menu() !== null}
      onOpenChange={(o) => !o && closeMenu()}
      x={menu()?.x ?? 0}
      y={menu()?.y ?? 0}
      items={menu()?.items ?? []}
      minWidth={menu()?.minWidth}
      listbox={menu()?.listbox}
      onPick={(it) => {
        closeMenu();
        void it.run?.();
      }}
    />
  );
}
