import { Show, type JSX } from "solid-js";
import { Button } from "./ui";
import { Icon } from "./Icon";

/** An empty list or panel: what is missing, why, and one way out. Use it instead of a bare "Sem X" line. */
export function EmptyState(props: {
  title: string;
  /** why it is empty, or what to do about it */
  why?: JSX.Element;
  action?: { label: string; onClick: () => void };
  icon?: string;
  class?: string;
}) {
  return (
    <div class={`placeholder empty ${props.class ?? ""}`} role="status">
      <Show when={props.icon}>{(i) => <Icon name={i()} size={20} />}</Show>
      <div class="font-medium text-foreground">{props.title}</div>
      <Show when={props.why}>
        <div class="auto-note">{props.why}</div>
      </Show>
      <Show when={props.action}>
        {(a) => (
          <div class="empty-actions">
            <Button variant="default" onClick={() => a().onClick()}>
              {a().label}
            </Button>
          </div>
        )}
      </Show>
    </div>
  );
}
