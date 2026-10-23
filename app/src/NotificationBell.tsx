import { For, Show, createSignal, onCleanup } from "solid-js";
import { moveIndex } from "./a11y";
import { isTopModal, popModal, pushModal } from "./modalStack";
import { NotifEntry, NotifKind, clearAll, markAllRead, notifications, unreadCount } from "./notifications";
import { ago } from "./time";
import "./notifications.css";
import { Button, cn } from "./ui";

/** word and glyph per kind: the meaning never rests on colour alone */
const KIND: Record<NotifKind, [string, string]> = {
  done: ["✓", "Terminou"],
  waiting: ["?", "Espera por ti"],
  "ci-fail": ["✕", "CI a falhar"],
  "ci-pass": ["✓", "CI a passar"],
  merged: ["⇄", "Merge feito"],
  error: ["!", "Erro"],
};

const FOCUSABLE = "button:not(:disabled)";

const wtName = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? "";

/** Bell with an unread badge and the list of recent events. `onJump` opens what an entry is about (its worktree, and
 *  its agent session when it has one). Mount once, e.g. in the sidebar header. */
export function NotificationBell(props: { onJump: (e: NotifEntry) => void }) {
  const [open, setOpen] = createSignal(false);
  // the entries that were unread when the list opened: still marked "novo" while it is open
  const [fresh, setFresh] = createSignal<Set<string>>(new Set());
  let bell!: HTMLButtonElement;
  let pop: HTMLDivElement | undefined;
  let layer: symbol | null = null;

  const close = (refocus = true) => {
    if (!open()) return;
    setOpen(false);
    if (layer) popModal(layer);
    layer = null;
    window.removeEventListener("keydown", onKey, true);
    document.removeEventListener("mousedown", onDown, true);
    if (refocus) bell.focus();
  };
  const onKey = (e: KeyboardEvent) => {
    if (!layer || !isTopModal(layer)) return;
    if (e.key === "Escape") {
      e.stopImmediatePropagation();
      close();
    } else if (e.key === "Tab" && pop) {
      // Tab leaves the popover: it closes, like a menu, and focus goes on from the bell
      const els = [...pop.querySelectorAll<HTMLElement>(FOCUSABLE)];
      const at = els.indexOf(document.activeElement as HTMLElement);
      if ((!e.shiftKey && at === els.length - 1) || (e.shiftKey && at <= 0)) {
        e.preventDefault();
        close();
      }
    } else if ((e.key === "ArrowDown" || e.key === "ArrowUp") && pop) {
      const rows = [...pop.querySelectorAll<HTMLElement>(".notif-item")];
      const at = rows.indexOf(document.activeElement as HTMLElement);
      const next = moveIndex(at < 0 ? -1 : at, rows.length, e.key);
      if (next !== null) {
        e.preventDefault();
        rows[next]?.focus();
      }
    }
  };
  const onDown = (e: MouseEvent) => {
    const t = e.target as Node;
    if (pop?.contains(t) || bell.contains(t)) return;
    close(false);
  };

  const show = () => {
    setFresh(new Set(notifications().filter((n) => !n.read).map((n) => n.id)));
    markAllRead();
    setOpen(true);
    layer = pushModal();
    window.addEventListener("keydown", onKey, true);
    document.addEventListener("mousedown", onDown, true);
    queueMicrotask(() => (pop?.querySelector<HTMLElement>(".notif-item") ?? pop?.querySelector<HTMLElement>(FOCUSABLE))?.focus());
  };
  onCleanup(() => close(false));

  const jump = (e: NotifEntry) => {
    close(false);
    props.onJump(e);
  };

  const label = () => {
    const n = unreadCount();
    return n ? `Notificações, ${n} por ler` : "Notificações";
  };

  return (
    <div class="notif">
      <Button ref={bell} variant="icon" size="glyph" class="relative" title="Notificações" aria-label={label()} aria-haspopup="dialog" aria-expanded={open()} onClick={() => (open() ? close() : show())}>
        <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
          <path d="M8 2.2a3.8 3.8 0 0 0-3.8 3.8c0 3-1.4 4-1.4 4h10.4s-1.4-1-1.4-4A3.8 3.8 0 0 0 8 2.2zM6.6 12.6a1.5 1.5 0 0 0 2.8 0" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
        <Show when={unreadCount() > 0}>
          <span class="notif-badge" aria-hidden="true">{unreadCount() > 99 ? "99+" : unreadCount()}</span>
        </Show>
      </Button>
      <Show when={open()}>
        <div ref={pop} class="notif-pop" role="dialog" aria-label="Notificações">
          <div class="notif-head">
            <span class="notif-title">Notificações</span>
            <Button variant="outline" size="sm" class="h-auto py-0.5" disabled={notifications().length === 0} onClick={() => clearAll()}>Limpar tudo</Button>
          </div>
          <Show when={notifications().length} fallback={<div class="notif-empty">Sem notificações</div>}>
            <ul class="notif-list">
              <For each={notifications()}>
                {(n) => (
                  <li>
                    <Button variant="row" size="bare" class={cn("notif-item h-auto w-full flex-col items-stretch gap-0.5 rounded-none border-0 border-b border-border px-2.5 py-2 hover:bg-muted focus-visible:bg-muted", fresh().has(n.id) && "border-l-[3px] border-l-primary pl-[7px]")} onClick={() => jump(n)}>
                      <span class={`notif-kind ${n.kind}`}>
                        <span aria-hidden="true">{KIND[n.kind][0]}</span> {KIND[n.kind][1]}
                        <Show when={fresh().has(n.id)}> · novo</Show>
                      </span>
                      <span class="notif-text">{n.text}</span>
                      <span class="notif-meta">
                        <Show when={wtName(n.worktree)}>{wtName(n.worktree)} · </Show>
                        {ago((Date.now() - n.at) / 1000)}
                      </span>
                    </Button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </div>
      </Show>
    </div>
  );
}
