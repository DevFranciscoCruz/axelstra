import { createSignal } from "solid-js";
import type { ITheme } from "@xterm/xterm";

export type TermTheme = "dark" | "light";
/** `screenReader`: xterm mirrors its output into an accessible DOM tree (costs speed, so it is off by default) */
export type TermPrefs = { theme: TermTheme; fontFamily: string; fontSize: number; screenReader: boolean };

// same stack as --font-mono-term in styles.css (xterm needs the literal string)
export const DEFAULT_FONT = '"Cascadia Mono", Consolas, monospace';
export const DEFAULT_PREFS: TermPrefs = { theme: "dark", fontFamily: DEFAULT_FONT, fontSize: 13, screenReader: false };
export const MIN_FONT = 8;
export const MAX_FONT = 32;

export const THEMES: Record<TermTheme, ITheme> = {
  dark: { background: "#1e1f23", foreground: "#d7dae0", cursor: "#a78bfa", selectionBackground: "#4b4370" },
  light: {
    background: "#fafafa",
    foreground: "#24292f",
    cursor: "#7c3aed",
    selectionBackground: "#c9bff5",
    black: "#24292f",
    red: "#cf222e",
    green: "#116329",
    yellow: "#9a6700",
    blue: "#0550ae",
    magenta: "#8250df",
    cyan: "#1b7c83",
    white: "#69717b",
    brightBlack: "#57606a",
    brightRed: "#a40e26",
    brightGreen: "#1a7f37",
    brightYellow: "#7d4e00",
    brightBlue: "#0969da",
    brightMagenta: "#8250df",
    brightCyan: "#17717f",
    brightWhite: "#5f6770",
  },
};

export const clampFont = (n: number) => (Number.isFinite(n) ? Math.min(MAX_FONT, Math.max(MIN_FONT, Math.round(n))) : DEFAULT_PREFS.fontSize);

/** tolerant: anything missing or malformed falls back to the default */
export const parsePrefs = (raw: string | null | undefined): TermPrefs => {
  try {
    const o = JSON.parse(raw ?? "{}") as Partial<TermPrefs>;
    return {
      theme: o.theme === "light" ? "light" : "dark",
      fontFamily: typeof o.fontFamily === "string" && o.fontFamily.trim() ? o.fontFamily.trim() : DEFAULT_FONT,
      fontSize: typeof o.fontSize === "number" ? clampFont(o.fontSize) : DEFAULT_PREFS.fontSize,
      screenReader: o.screenReader === true,
    };
  } catch {
    return { ...DEFAULT_PREFS };
  }
};

/** the cursor blinks only when the user has not asked the system for less motion */
export const cursorBlinks = () => {
  try {
    return !matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return true;
  }
};

/** font size after a zoom key: `+1`, `-1`, or `0` for the default */
export const zoomFont = (cur: number, delta: 1 | -1 | 0) => (delta === 0 ? DEFAULT_PREFS.fontSize : clampFont(cur + delta));

// kept in the webview: the daemon's settings file only knows the keys it validates (see settings.set)
const KEY = "axelstra.terminal";
const load = () => {
  try {
    return parsePrefs(localStorage.getItem(KEY));
  } catch {
    return { ...DEFAULT_PREFS };
  }
};
export const [termPrefs, setPrefsSig] = createSignal<TermPrefs>(load());
export const setTermPrefs = (patch: Partial<TermPrefs>) => {
  const next = parsePrefs(JSON.stringify({ ...termPrefs(), ...patch }));
  setPrefsSig(next);
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    /* storage unavailable: the choice lasts until the window closes */
  }
};
