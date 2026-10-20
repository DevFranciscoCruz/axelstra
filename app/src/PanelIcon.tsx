import { Show } from "solid-js";

/** window-with-side-panel glyph; the panel strip is filled while that panel is shown */
export const PanelIcon = (props: { side: "left" | "right"; on: boolean }) => (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round">
    <rect x="2" y="2.5" width="12" height="11" rx="2.5" />
    <path d={props.side === "left" ? "M6.2 2.5v11" : "M9.8 2.5v11"} />
    <Show when={props.on}>
      <path
        d={props.side === "left" ? "M4.5 2.5H6.2v11H4.5a2.5 2.5 0 0 1-2.5-2.5V5a2.5 2.5 0 0 1 2.5-2.5z" : "M11.5 2.5H9.8v11h1.7a2.5 2.5 0 0 0 2.5-2.5V5a2.5 2.5 0 0 0-2.5-2.5z"}
        fill="currentColor"
        stroke="none"
      />
    </Show>
  </svg>
);
