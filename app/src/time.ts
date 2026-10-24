import { fmtDate, fmtRelative } from "./i18n";

/** How long ago, in the language of the UI through `Intl.RelativeTimeFormat` ("agora", "há 5 min", "há 3 h", "há 3 dias";
 *  "now", "5 min. ago"...). Under a minute is "now"; past a day it counts days. */
export const ago = (seconds: number): string => {
  const s = Math.max(0, seconds);
  if (s < 60) return fmtRelative(0, "second", "short");
  if (s < 3600) return fmtRelative(-Math.floor(s / 60), "minute", "short");
  if (s < 86400) return fmtRelative(-Math.floor(s / 3600), "hour", "short");
  return fmtRelative(-Math.floor(s / 86400), "day", "short");
};

/** A moment (Unix seconds) the way the UI language writes dates: "24/10/2026, 13:05", "Oct 24, 2026, 1:05 PM". */
export const stampLocal = (at: number): string => fmtDate(at * 1000);

const pad = (n: number) => String(n).padStart(2, "0");
/** "2026-10-24 13:05" in local time, whatever the UI language: for files that are exported or read by other tools. */
export const stampIso = (at: number): string => {
  const d = new Date(at * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
};
