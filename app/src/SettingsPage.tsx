import { For, JSX, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { Icon } from "./Icon";
import { K } from "./keyboard";
import { menuOpen } from "./ContextMenu";
import { Button, Input, cn } from "./ui";
import { L } from "./strings.logic";
import { GROUPS, SectionId, Caps, effectiveSection, section, setSection, visibleItems } from "./settingsNav";

/** Esc leaves the page unless something above it (a dialog, the palette, a menu or Select) owns the key. */
export const escapeLeaves = (e: Pick<KeyboardEvent, "key" | "defaultPrevented">): boolean =>
  e.key === "Escape" &&
  !e.defaultPrevented &&
  !menuOpen() &&
  !document.querySelector('[role="dialog"], [role="alertdialog"], .pal-back');

/**
 * The settings as a page of the app, not an overlay: a side list of sections (grouped, searchable, collapsing in a narrow
 * window) and one section at a time on the right. It changes no session: closing only hides the page, and focus goes back
 * to where it was. `page` draws the section asked for.
 */
export function SettingsPage(props: { caps: Caps; onClose: () => void; page: (id: SectionId) => JSX.Element }) {
  const T = L.settingsNav;
  const [query, setQuery] = createSignal("");
  const [navOpen, setNavOpen] = createSignal(false);
  const items = createMemo(() => visibleItems(props.caps, query()));
  const current = createMemo(() => effectiveSection(section(), props.caps));
  const head = createMemo(() => visibleItems(props.caps, "").find((i) => i.id === current()));
  let search: HTMLInputElement | undefined;
  let nav: HTMLElement | undefined;
  const from = document.activeElement as HTMLElement | null;

  const pick = (id: SectionId) => {
    setSection(id);
    setNavOpen(false);
    document.getElementById("set-content")?.scrollTo?.({ top: 0 });
  };
  const onKey = (e: KeyboardEvent) => {
    if (escapeLeaves(e)) {
      e.preventDefault();
      if (query()) setQuery("");
      else props.onClose();
    } else if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "f") {
      e.preventDefault();
      e.stopPropagation();
      setNavOpen(true);
      search?.focus();
      search?.select();
    }
  };
  onMount(() => {
    document.addEventListener("keydown", onKey);
    (nav?.querySelector<HTMLElement>('[aria-current="page"]') ?? search)?.focus();
  });
  onCleanup(() => {
    document.removeEventListener("keydown", onKey);
    // back to where the user was (the terminal, a tab), unless that is gone or the user already put focus elsewhere
    if (
      from?.isConnected &&
      (!document.activeElement || document.activeElement === document.body || nav?.contains(document.activeElement))
    )
      from.focus();
  });

  return (
    <div class="set-page" data-testid="settings-page">
      <div class="set-side">
        <Button variant="ghost" class="set-back" onClick={props.onClose}>
          <Icon name="arrowLeft" size={14} inline /> {T.back}
        </Button>
        <div class="set-search">
          <Icon name="search" size={14} />
          <Input
            ref={search}
            type="search"
            value={query()}
            placeholder={T.search}
            aria-label={T.search}
            aria-keyshortcuts="Control+F Meta+F"
            autocomplete="off"
            spellcheck={false}
            onInput={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && items().length === 1) pick(items()[0].id);
            }}
          />
          <kbd>{K("Ctrl+F")}</kbd>
        </div>
        <Button
          variant="outline"
          class="set-nav-toggle"
          aria-expanded={navOpen()}
          aria-controls="set-nav"
          onClick={() => setNavOpen(!navOpen())}
        >
          <Icon name={navOpen() ? "chevDown" : "chevRight"} size={12} inline /> {T.sections}: {head()?.title}
        </Button>
        <nav id="set-nav" class="set-nav" classList={{ open: navOpen() }} aria-label={T.navLabel} ref={nav}>
          <Show when={items().length} fallback={<p class="auto-note">{T.noMatch}</p>}>
            <For each={GROUPS}>
              {(g) => (
                <Show when={items().some((i) => i.group === g)}>
                  <h3 class="set-group" id={`set-group-${g}`}>
                    {T.groups[g]}
                  </h3>
                  <ul aria-labelledby={`set-group-${g}`}>
                    <For each={items().filter((i) => i.group === g)}>
                      {(i) => (
                        <li>
                          <Button
                            variant="row"
                            class={cn("set-nav-item", i.id === current() && "on")}
                            aria-current={i.id === current() ? "page" : undefined}
                            onClick={() => pick(i.id)}
                          >
                            <Icon name={i.icon} size={14} /> {i.title}
                          </Button>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
              )}
            </For>
          </Show>
        </nav>
      </div>
      <div class="set-content" id="set-content" role="region" aria-labelledby="set-title">
        <div class="set-inner">
          <h2 id="set-title">{head()?.title}</h2>
          <p class="set-sub">{head()?.sub}</p>
          {props.page(current())}
        </div>
      </div>
    </div>
  );
}
