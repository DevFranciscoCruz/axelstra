import { JSX } from "solid-js";

/** Stroke icons that replace font glyphs (×, ⌄, ↻ ...): a glyph sits on the text baseline and looks off-centre
 *  next to text in a different font, an SVG centres exactly. Sized in em so it scales with the button. */
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
  gemini: <path d="M8 1.5C8.6 5.2 10.8 7.4 14.5 8 10.8 8.6 8.6 10.8 8 14.5 7.4 10.8 5.2 8.6 1.5 8 5.2 7.4 7.4 5.2 8 1.5z" />,
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
  gear: (
    <>
      <circle cx="8" cy="8" r="2" />
      <path d="M8 1.8v1.7M8 12.5v1.7M1.8 8h1.7M12.5 8h1.7M3.6 3.6l1.2 1.2M11.2 11.2l1.2 1.2M12.4 3.6l-1.2 1.2M4.8 11.2l-1.2 1.2" />
    </>
  ),
};

/** JSX builds real DOM nodes: inserting the shared node in a second place would move it out of the first,
 *  leaving every other use of the icon empty. Each use gets its own copy. */
const fresh = (el: JSX.Element): JSX.Element => (Array.isArray(el) ? el.map(fresh) : el instanceof Node ? el.cloneNode(true) : el);

/** true when `name` is an icon this file draws (menus accept an icon name or plain text) */
export const hasIcon = (name: string | undefined): boolean => !!name && name in PATHS;

export const Icon = (props: { name: keyof typeof PATHS | string; size?: number; inline?: boolean }) => (
  <svg
    class="svg-icon"
    classList={{ inline: props.inline }}
    width={props.size ?? 13}
    height={props.size ?? 13}
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    {fresh(PATHS[props.name])}
  </svg>
);
