/** How long ago, in the one wording the app uses ("agora", "há 5 min", "há 3 h", "há 2 d"). */
export const ago = (seconds: number): string => {
  const s = Math.max(0, seconds);
  if (s < 60) return "agora";
  if (s < 3600) return `há ${Math.floor(s / 60)} min`;
  if (s < 86400) return `há ${Math.floor(s / 3600)} h`;
  return `há ${Math.floor(s / 86400)} d`;
};
