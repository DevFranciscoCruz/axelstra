import { For, Show } from "solid-js";
import { Icon } from "../Icon";
import { Button, Input, cn } from "../ui";
import { MenuItem, isMenuKey, showMenu, showMenuAt } from "../ContextMenu";
import { P } from "../strings.panels";
import { ProjectIcon } from "./ProjectIcon";
import { HostBadge } from "../HostBadge";
import { displayPath, hostOf } from "../paths";

const T = P.sidebar;
type Proj = {
  id: string;
  name: string;
  path: string;
  slug?: string | null;
  host?: string;
  icon_url?: string | null;
  setup?: string | null;
};

/** The header of a project: fold toggle, icon, name and its buttons; the menu opens from right click or the menu key. */
export function ProjectHead(props: {
  project: Proj;
  folded: boolean;
  menu: () => MenuItem[];
  onFold: () => void;
  onNew: () => void;
  onSetup: () => void;
  onDrop: () => void;
}) {
  const p = () => props.project;
  return (
    <div
      class="project-head"
      title={displayPath(p().path)}
      onContextMenu={(e) => showMenu(e, props.menu())}
      onKeyDown={(e) => {
        // the menu key and Shift+F10 open the project menu from any button of the header
        if (!isMenuKey(e)) return;
        e.preventDefault();
        const r = e.currentTarget.getBoundingClientRect();
        showMenuAt(r.left + 24, r.bottom, props.menu());
      }}
    >
      <Button
        type="button"
        variant="icon"
        size="icon"
        class="size-5 flex-none opacity-80"
        aria-expanded={!props.folded}
        aria-label={props.folded ? T.openProject(p().name) : T.closeProject(p().name)}
        title={props.folded ? T.showWorktrees : T.hideWorktrees}
        onClick={props.onFold}
      >
        <Icon name={props.folded ? "chevRight" : "chevDown"} size={12} />
      </Button>
      <ProjectIcon id={p().id} name={p().name} url={p().icon_url} />
      <span class="pname" title={p().slug ?? displayPath(p().path)}>
        {p().name}
      </span>
      <Show when={p().host ?? hostOf(p().path)}>{(h) => <HostBadge host={h()} />}</Show>
      <Button variant="icon" size="glyph" title={T.newWorktree} aria-label={T.newWorktree} onClick={props.onNew}>
        <Icon name="plus" />
      </Button>
      <Button
        variant="icon"
        size="glyph"
        title={T.setupTip(p().setup)}
        aria-label={T.setupTip(p().setup)}
        onClick={props.onSetup}
      >
        <Icon name="gear" />
      </Button>
      <Button variant="icon" size="glyph" title={T.dropTip} aria-label={T.dropLabel} onClick={props.onDrop}>
        <Icon name="x" />
      </Button>
    </div>
  );
}

/** The inline setup-command editor under a project header, with the package managers found on this machine as one-click commands. */
export function SetupEditor(props: {
  name: string;
  text: string;
  setText: (v: string) => void;
  pms: { installed: string[]; suggested: string | null };
  onSave: () => void;
  onClose: () => void;
}) {
  return (
    <>
      <Input
        class="mx-1.5 mb-1.5 mt-0.5 w-[calc(100%-12px)] border-primary bg-background"
        placeholder={T.setupPlaceholder}
        aria-label={T.setupLabel(props.name)}
        value={props.text}
        onInput={(e) => props.setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") props.onSave();
          if (e.key === "Escape") props.onClose();
        }}
        ref={(el) => queueMicrotask(() => el.focus())}
      />
      <Show when={props.pms.installed.length > 0}>
        <div class="pm-row">
          <For each={props.pms.installed}>
            {(m) => (
              <Button
                variant="secondary"
                size="pill"
                class={cn(
                  "border-border px-2.5 py-0.5 text-sm",
                  props.text === `${m} install` && "border-primary text-primary",
                )}
                title={m === props.pms.suggested ? T.pmSuggested : T.pmUse(m)}
                onClick={() => props.setText(`${m} install`)}
              >
                {m}
                {m === props.pms.suggested ? <Icon name="star" size={10} inline /> : null}
              </Button>
            )}
          </For>
        </div>
      </Show>
    </>
  );
}
