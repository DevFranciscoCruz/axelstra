import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { Icon, hasIcon } from "./Icon";

export type MenuItem =
  | { separator: true }
  | {
      label: string;
      icon?: string;
      run?: () => unknown;
      danger?: boolean;
      disabled?: boolean;
      /** submenu; opens on hover */
      children?: MenuItem[];
    };

type Open = { x: number; y: number; items: MenuItem[]; minWidth?: number };
const [menu, setMenu] = createSignal<Open | null>(null);

/** Right-click menu at the pointer. */
export const showMenu = (e: MouseEvent, items: MenuItem[]) => {
  e.preventDefault();
  e.stopPropagation();
  setMenu({ x: e.clientX, y: e.clientY, items });
};

/** Menu at a fixed point (dropdowns open under their button). */
export const showMenuAt = (x: number, y: number, items: MenuItem[], minWidth?: number) => setMenu({ x, y, items, minWidth });

function Items(props: { items: MenuItem[]; sub?: boolean; flip?: boolean }) {
  const [openSub, setOpenSub] = createSignal<number | null>(null);
  const [flip, setFlip] = createSignal(false);
  return (
    <div class="ctx" role="menu" classList={{ sub: props.sub, flip: props.sub && props.flip }} style={{ "min-width": "inherit" }}>
      <For each={props.items}>
        {(it, i) =>
          "separator" in it ? (
            <div class="ctx-sep" />
          ) : (
            <div
              class="ctx-wrap"
              onMouseEnter={(e) => {
                setOpenSub(it.children ? i() : null);
                setFlip(e.currentTarget.getBoundingClientRect().right + 230 > window.innerWidth);
              }}
            >
              <button
                class="ctx-item"
                role="menuitem"
                aria-haspopup={it.children ? "menu" : undefined}
                classList={{ danger: it.danger, off: it.disabled }}
                disabled={it.disabled}
                onClick={(e) => {
                  e.stopPropagation();
                  if (it.children) return;
                  setMenu(null);
                  void it.run?.();
                }}
              >
                <span class="ctx-icon">{hasIcon(it.icon) ? <Icon name={it.icon!} size={13} /> : (it.icon ?? "")}</span>
                <span class="ctx-label">{it.label}</span>
                <Show when={it.children}>
                  <span class="ctx-arrow"><Icon name="chevRight" size={11} /></span>
                </Show>
              </button>
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
    const close = () => setMenu(null);
    const key = (e: KeyboardEvent) => {
      if (!menu()) return;
      if (e.key === "Escape") return close();
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
            <Items items={m().items} />
          </div>
        );
      }}
    </Show>
  );
}
