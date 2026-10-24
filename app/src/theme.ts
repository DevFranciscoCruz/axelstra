import { createSignal } from "solid-js";
import { readString, writeString } from "./persist";
import { setTermPrefs } from "./termprefs";

export type AppTheme = "auto" | "dark" | "light";
export const APP_THEMES: AppTheme[] = ["auto", "dark", "light"];
const KEY = "axelstra.theme";

const stored = (): AppTheme => {
  const v = readString(KEY);
  return v === "dark" || v === "light" ? v : "auto";
};

const systemLight = () => {
  try {
    return window.matchMedia("(prefers-color-scheme: light)").matches;
  } catch {
    return false;
  }
};

export const [appTheme, setAppThemeSig] = createSignal<AppTheme>(stored());
/** the theme in use once "auto" follows the system */
export const [isLight, setIsLight] = createSignal(false);

const apply = () => {
  const light = appTheme() === "light" || (appTheme() === "auto" && systemLight());
  setIsLight(light);
  document.documentElement.dataset.theme = light ? "light" : "dark";
};

export const setAppTheme = (t: AppTheme) => {
  setAppThemeSig(t);
  writeString(KEY, t); // storage unavailable: the choice lasts until restart
  apply();
  syncTerminal();
};

/** terminals follow the app theme when it changes (they can still be set apart in Settings afterwards) */
const syncTerminal = () => setTermPrefs({ theme: isLight() ? "light" : "dark" });

/** call once at startup, before the first render so there is no flash of the wrong theme */
export const initTheme = () => {
  apply();
  try {
    window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", () => {
      apply();
      if (appTheme() === "auto") syncTerminal();
    });
  } catch {
    /* no matchMedia */
  }
};
