import { JSX, Show } from "solid-js";
import { Icon } from "./Icon";

/** One tab of the strip: a tab button, plus a separate close button beside it (a button never sits inside another).
 *  The wrapper takes the drag and mouse handlers; `editing` replaces the label while the name is being edited. */
export function Tab(props: {
  selected: boolean;
  /** in the tab order: the selected tab, or the one that stands in when none is */
  tabbable?: boolean;
  onSelect: () => void;
  onClose?: () => void;
  closeLabel?: string;
  onRename?: () => void;
  title?: string;
  id?: string;
  class?: string;
  wrap?: JSX.HTMLAttributes<HTMLDivElement>;
  editing?: JSX.Element;
  children: JSX.Element;
}) {
  return (
    <div class={`tab ${props.class ?? ""}`} classList={{ active: props.selected }} role="presentation" {...props.wrap}>
      <Show
        when={props.editing}
        fallback={
          <button
            type="button"
            class="tab-main"
            role="tab"
            id={props.id}
            data-tab={props.id}
            aria-selected={props.selected}
            tabindex={props.selected || props.tabbable ? 0 : -1}
            title={props.title}
            onClick={props.onSelect}
            onKeyDown={(e) => {
              if (e.key === "F2" && props.onRename) {
                e.preventDefault();
                props.onRename();
              }
            }}
          >
            {props.children}
          </button>
        }
      >
        {props.editing}
      </Show>
      <Show when={props.onClose}>
        <button type="button" class="x" title={props.closeLabel ?? "Fechar"} aria-label={props.closeLabel ?? "Fechar"} onClick={() => props.onClose!()}>
          <Icon name="x" />
        </button>
      </Show>
    </div>
  );
}
