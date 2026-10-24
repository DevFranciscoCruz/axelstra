import { JSX, Show } from "solid-js";
import { Icon } from "../Icon";
import { sessions, setError, showNotice } from "../store";
import { P } from "../strings.panels";

export type DTab = "info" | "logs" | "terminal" | "files";
export const DTABS: [DTab, string][] = (["info", "logs", "terminal", "files"] as DTab[]).map((t) => [
  t,
  P.docker.tabs[t],
]);

/** Small stroke icons the shared Icon set does not have. */
const GLYPHS: Record<string, JSX.Element> = {
  stop: <rect x="4" y="4" width="8" height="8" rx="1" />,
  trash: <path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8h5.8l.6-8" />,
  terminal: <path d="M2.5 4l4 4-4 4M8 12h5.5" />,
  logs: <path d="M3 4h10M3 8h10M3 12h6" />,
};
export function DIcon(props: { name: string }) {
  return (
    <Show when={GLYPHS[props.name]} fallback={<Icon name={props.name} size={14} />}>
      <svg
        class="svg-icon"
        width="14"
        height="14"
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        stroke-width="1.4"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        {GLYPHS[props.name]}
      </svg>
    </Show>
  );
}

/** results go to the app toast, like everywhere else */
export const say = (text: string, err = false) => (err ? setError(text) : showNotice(text));
export const nameOf = (e: unknown) => String(e).replace(/^Error: /, "");

/** shell sessions started from the detail pane, per container id, so coming back to the tab finds the same shell */
export const shells = new Map<string, number>();
export const FAST_MS = 3000;
