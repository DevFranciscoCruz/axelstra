import { JSX } from "solid-js";

/** Stroke icons that replace font glyphs (×, ⌄, ↻ ...): a glyph sits on the text baseline and looks off-centre
 *  next to text in a different font, an SVG centres exactly. Every icon of the app is drawn here (the lint in
 *  uiRules.test.ts forbids a `<svg` anywhere else): one 16 box, one stroke, three sizes (see `snap`). */
const PATHS: Record<string, JSX.Element> = {
  x: <path d="M4 4l8 8M12 4l-8 8" />,
  // agent marks, drawn as simple strokes
  claude: <path d="M8 2v12M2 8h12M3.8 3.8l8.4 8.4M12.2 3.8l-8.4 8.4" />,
  codex: (
    <>
      <path d="M8 1.8l5.4 3.1v6.2L8 14.2l-5.4-3.1V4.9z" />
      <path d="M8 1.8v6.2l5.4 3.1M8 8L2.6 11.1M2.6 4.9L8 8l5.4-3.1" />
    </>
  ),
  gemini: (
    <path d="M8 1.5C8.6 5.2 10.8 7.4 14.5 8 10.8 8.6 8.6 10.8 8 14.5 7.4 10.8 5.2 8.6 1.5 8 5.2 7.4 7.4 5.2 8 1.5z" />
  ),
  plus: <path d="M8 3v10M3 8h10" />,
  check: <path d="M3.5 8.5l3 3 6-7" />,
  chevDown: <path d="M4 6l4 4 4-4" />,
  chevRight: <path d="M6 4l4 4-4 4" />,
  more: (
    <>
      <circle cx="3.5" cy="8" r=".9" fill="currentColor" />
      <circle cx="8" cy="8" r=".9" fill="currentColor" />
      <circle cx="12.5" cy="8" r=".9" fill="currentColor" />
    </>
  ),
  refresh: (
    <>
      <path d="M13 8a5 5 0 1 1-1.6-3.7" />
      <path d="M13 2.5v3h-3" />
    </>
  ),
  external: (
    <>
      <path d="M9 3h4v4M13 3L7.5 8.5" />
      <path d="M11 9.5V12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h2.5" />
    </>
  ),
  play: <path d="M5 3.5v9l7-4.5z" fill="currentColor" />,
  bolt: <path d="M9 2L4.5 9H8l-1 5L11.5 7H8z" />,
  pause: <path d="M5.5 3.5v9M10.5 3.5v9" />,
  pencil: <path d="M10.5 3l2.5 2.5L6 12.5 3 13l.5-3z" />,
  arrowUp: <path d="M8 13V3M4 7l4-4 4 4" />,
  arrowDown: <path d="M8 3v10M4 9l4 4 4-4" />,
  arrowRight: <path d="M3 8h10M9 4l4 4-4 4" />,
  subArrow: <path d="M4 3v5a2 2 0 0 0 2 2h6M9 7l3 3-3 3" />,
  copy: (
    <>
      <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
      <path d="M10.5 5.5v-2a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2" />
    </>
  ),
  folder: <path d="M2 4.5a1 1 0 0 1 1-1h3l1.5 1.5H13a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1z" />,
  file: <path d="M4 2.5h5l3 3v8H4zM9 2.5v3h3" />,
  box: <path d="M2.5 5.5 8 2.5l5.5 3v5L8 13.5l-5.5-3zM2.5 5.5 8 8.5l5.5-3M8 8.5v5" />,
  grid: (
    <>
      <rect x="2.5" y="2.5" width="4.5" height="4.5" rx=".8" />
      <rect x="9" y="2.5" width="4.5" height="4.5" rx=".8" />
      <rect x="2.5" y="9" width="4.5" height="4.5" rx=".8" />
      <rect x="9" y="9" width="4.5" height="4.5" rx=".8" />
    </>
  ),
  pin: <path d="M9.5 2.5l4 4-2 1-2 2 .5 2.5-1 1-2.7-2.7L3 14M6.3 9.7L4 7.4l1-1 2.5.5 2-2z" />,
  moon: <path d="M13 9.5A5.5 5.5 0 1 1 6.5 3a4.5 4.5 0 0 0 6.5 6.5z" />,
  circleHalf: (
    <>
      <circle cx="8" cy="8" r="5.5" />
      <path d="M8 8V2.5A5.5 5.5 0 0 1 13.5 8z" fill="currentColor" />
    </>
  ),
  warning: <path d="M8 2.5l6 10.5H2zM8 6.5v3M8 11.3v.1" />,
  star: <path d="M8 2l1.8 3.7 4 .6-2.9 2.8.7 4L8 11.2 4.4 13.1l.7-4L2.2 6.3l4-.6z" fill="currentColor" />,
  clock: (
    <>
      <circle cx="8" cy="8" r="5.5" />
      <path d="M8 5v3l2 1.5" />
    </>
  ),
  mail: (
    <>
      <rect x="2" y="3.5" width="12" height="9" rx="1.5" />
      <path d="M2.5 4.5L8 9l5.5-4.5" />
    </>
  ),
  branch: (
    <>
      <circle cx="4.5" cy="3.5" r="1.5" />
      <circle cx="4.5" cy="12.5" r="1.5" />
      <circle cx="11.5" cy="5.5" r="1.5" />
      <path d="M4.5 5v6M11.5 7c0 2.5-3 2.5-6.5 4" />
    </>
  ),
  sparkle: <path d="M8 2l1.4 4.6L14 8l-4.6 1.4L8 14l-1.4-4.6L2 8l4.6-1.4z" />,
  link: <path d="M6.5 9.5l3-3M7 5l1-1a2.5 2.5 0 0 1 3.5 3.5l-1 1M9 11l-1 1A2.5 2.5 0 0 1 4.5 8.5l1-1" />,
  triUp: <path d="M8 4l4.5 7h-9z" fill="currentColor" />,
  triDown: <path d="M8 12L3.5 5h9z" fill="currentColor" />,
  dot: <circle cx="8" cy="8" r="3" fill="currentColor" />,
  ellipsis: <path d="M3 8h.1M8 8h.1M13 8h.1" stroke-width="2.2" />,
  coffee: <path d="M3 7h8v3a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V7zM11 8h1a1.5 1.5 0 0 1 0 3h-1M5 2.5v2M8 2.5v2" />,
  memory: (
    <>
      <rect x="2" y="5" width="12" height="6" rx="1" />
      <path d="M5 11v2M8 11v2M11 11v2M5 5V3M8 5V3M11 5V3" />
    </>
  ),
  cpu: (
    <>
      <rect x="4" y="4" width="8" height="8" rx="1" />
      <path d="M6.5 1.5v2.5M9.5 1.5v2.5M6.5 12v2.5M9.5 12v2.5M1.5 6.5H4M1.5 9.5H4M12 6.5h2.5M12 9.5h2.5" />
    </>
  ),
  gpu: (
    <>
      <rect x="1.5" y="4.5" width="13" height="7" rx="1" />
      <circle cx="8" cy="8" r="2" />
      <path d="M4 11.5v2M12 11.5v2" />
    </>
  ),
  plug: <path d="M6 2.5v3M10 2.5v3M4.5 5.5h7v2.5a3.5 3.5 0 0 1-7 0V5.5zM8 11.5v2" />,
  terminal: <path d="M2.5 4l4 4-4 4M8 12h5.5" />,
  logs: <path d="M3 4h10M3 8h10M3 12h6" />,
  stop: <rect x="4" y="4" width="8" height="8" rx="1" />,
  trash: <path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8h5.8l.6-8" />,
  split: <path d="M5.5 3v10M10.5 3v10" />,
  sparkleSolid: (
    <path d="M8 1.5c.4 3.4 1.6 4.6 5 5-3.4.4-4.6 1.6-5 5-.4-3.4-1.6-4.6-5-5 3.4-.4 4.6-1.6 5-5z" fill="currentColor" />
  ),
  collapse: <path d="M3 5h10M5 8.5h6M7 12h2" />,
  folderClosed: (
    <path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h2.6l1.4 1.5h5A1.5 1.5 0 0 1 14 6v5.5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z" />
  ),
  folderOpen: (
    <path d="M2 11.5V4.5A1.5 1.5 0 0 1 3.5 3h2.6l1.4 1.5h5A1.5 1.5 0 0 1 14 6v1.2M2.3 12.4l1.3-4.2a1 1 0 0 1 1-.7h9.2a.8.8 0 0 1 .8 1l-1.1 3.9a1.3 1.3 0 0 1-1.3.9H3.6a1.3 1.3 0 0 1-1.3-.9z" />
  ),
  fileDoc: <path d="M4 2h5l3 3v8.5a.5.5 0 0 1-.5.5h-7.5a.5.5 0 0 1-.5-.5v-11A.5.5 0 0 1 4 2zM9 2v3h3" />,
  bell: (
    <path d="M8 2.2a3.8 3.8 0 0 0-3.8 3.8c0 3-1.4 4-1.4 4h10.4s-1.4-1-1.4-4A3.8 3.8 0 0 0 8 2.2zM6.6 12.6a1.5 1.5 0 0 0 2.8 0" />
  ),
  mic: (
    <>
      <rect x="6" y="1.8" width="4" height="7.4" rx="2" />
      <path d="M3.5 7.6a4.5 4.5 0 0 0 9 0M8 12.2v2" />
    </>
  ),
  download: <path d="M8 2.5v8M4.8 7.6 8 10.8l3.2-3.2M3 13h10" />,
  // window with a side panel; the "On" ones have the strip filled (the panel is shown)
  panelLeft: (
    <>
      <rect x="2" y="2.5" width="12" height="11" rx="2.5" />
      <path d="M6.2 2.5v11" />
    </>
  ),
  panelLeftOn: (
    <>
      <rect x="2" y="2.5" width="12" height="11" rx="2.5" />
      <path d="M6.2 2.5v11" />
      <path d="M4.5 2.5H6.2v11H4.5a2.5 2.5 0 0 1-2.5-2.5V5a2.5 2.5 0 0 1 2.5-2.5z" fill="currentColor" stroke="none" />
    </>
  ),
  panelRight: (
    <>
      <rect x="2" y="2.5" width="12" height="11" rx="2.5" />
      <path d="M9.8 2.5v11" />
    </>
  ),
  panelRightOn: (
    <>
      <rect x="2" y="2.5" width="12" height="11" rx="2.5" />
      <path d="M9.8 2.5v11" />
      <path d="M11.5 2.5H9.8v11h1.7a2.5 2.5 0 0 0 2.5-2.5V5a2.5 2.5 0 0 0-2.5-2.5z" fill="currentColor" stroke="none" />
    </>
  ),
  // brand marks of the limits bar
  claudeMark: (
    <path
      d="M8 1l1.2 4.3L13.5 3l-2.3 4.2L15 8l-3.8.8 2.3 4.2-4.3-2.3L8 15l-1.2-4.3L2.5 13l2.3-4.2L1 8l3.8-.8L2.5 3l4.3 2.3z"
      fill="currentColor"
      stroke="none"
    />
  ),
  codexMark: <path d="M8 1.5l5.5 3.2v6.6L8 14.5l-5.5-3.2V4.7zM8 1.5v13M2.5 4.7l11 6.6M13.5 4.7l-11 6.6" />,
  gear: (
    <>
      <circle cx="8" cy="8" r="2" />
      <path d="M8 1.8v1.7M8 12.5v1.7M1.8 8h1.7M12.5 8h1.7M3.6 3.6l1.2 1.2M11.2 11.2l1.2 1.2M12.4 3.6l-1.2 1.2M4.8 11.2l-1.2 1.2" />
    </>
  ),
};

/** JSX builds real DOM nodes: inserting the shared node in a second place would move it out of the first,
 *  leaving every other use of the icon empty. Each use gets its own copy. */
const fresh = (el: JSX.Element): JSX.Element =>
  Array.isArray(el) ? el.map(fresh) : el instanceof Node ? el.cloneNode(true) : el;

/** true when `name` is an icon this file draws (menus accept an icon name or plain text) */
export const hasIcon = (name: string | undefined): boolean => !!name && name in PATHS;

/** The icon scale is 12 / 14 / 16: any size asked for up to 16 snaps to the nearest step above it. */
export const snap = (n: number): number => (n <= 12 ? 12 : n <= 14 ? 14 : n <= 16 ? 16 : n);

export const Icon = (props: { name: keyof typeof PATHS | string; size?: number; inline?: boolean; class?: string }) => (
  <svg
    class={`svg-icon${props.inline ? " inline" : ""}${props.class ? ` ${props.class}` : ""}`}
    width={snap(props.size ?? 14)}
    height={snap(props.size ?? 14)}
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.4"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    {fresh(PATHS[props.name])}
  </svg>
);
