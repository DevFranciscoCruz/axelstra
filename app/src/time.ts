import { L } from "./strings.logic";

/** How long ago, in the one wording the app uses ("agora", "há 5 min", "há 3 h", "há 2 d"; "now", "5 min ago"...). */
export const ago = (seconds: number): string => {
  const s = Math.max(0, seconds);
  if (s < 60) return L.ago.now;
  if (s < 3600) return L.ago.min(Math.floor(s / 60));
  if (s < 86400) return L.ago.hour(Math.floor(s / 3600));
  return L.ago.day(Math.floor(s / 86400));
};
