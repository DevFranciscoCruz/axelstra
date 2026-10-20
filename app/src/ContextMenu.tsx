import { For, Show, createSignal, onCleanup, onMount } from "solid-js";

export type MenuItem =
  | { separator: true }
  | {
      label: string;
      icon?: string;
      run?: () => void | Promise<void>;
      danger?: boolean;
      disabled?: boolean;
      /** submenu; opens on hover */
      children?: MenuItem[];
    };

type Open = { x: number; y: number; items: MenuItem[] };
const [menu, setMenu] = createSignal<Open | null>(null);

/** Right-click menu at the pointer. */
export const showMenu = (e: MouseEvent, items: MenuItem[]) => {
  e.preventDefault();
  e.stopPropagation();
  setMenu({ x: e.clientX, y: e.clientY, items });
};

function Items(props: { items: MenuItem[]; sub?: boolean }) {
  const [openSub, setOpenSub] = createSignal<number | null>(null);
  return (
    <div class="ctx" classList={{ sub: props.sub }}>
      <For each={props.items}>
        {(it, i) =>
          "separator" in it ? (
            <div class="ctx-sep" />
          ) : (
            <div class="ctx-wrap" onMouseEnter={() => setOpenSub(it.children ? i() : null)}>
              <button
                class="ctx-item"
                classList={{ danger: it.danger, off: it.disabled }}
                disabled={it.disabled}
                onClick={(e) => {
                  e.stopPropagation();
                  if (it.children) return;
                  setMenu(null);
                  void it.run?.();
                }}
              >
                <span class="ctx-icon">{it.icon ?? ""}</span>
                <span class="ctx-label">{it.label}</span>
                <Show when={it.children}>
                  <span class="ctx-arrow">›</span>
                </Show>
              </button>
              <Show when={it.children && openSub() === i()}>
                <Items items={it.children!} sub />
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
    const key = (e: KeyboardEvent) => e.key === "Escape" && close();
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
          <div class="ctx-root" style={{ left: `${left()}px`, top: `${top()}px` }} onContextMenu={(e) => e.preventDefault()}>
            <Items items={m().items} />
          </div>
        );
      }}
    </Show>
  );
}
