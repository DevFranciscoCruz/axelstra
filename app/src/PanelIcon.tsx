import { Icon } from "./Icon";

/** window-with-side-panel glyph; the panel strip is filled while that panel is shown */
export const PanelIcon = (props: { side: "left" | "right"; on: boolean }) => (
  <Icon name={`panel${props.side === "left" ? "Left" : "Right"}${props.on ? "On" : ""}`} size={16} />
);
