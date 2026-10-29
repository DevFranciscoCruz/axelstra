import { G } from "./strings.memory";

/** A size in MB for the screen ("512 MB", "1,5 GB"). */
export const fmtMb = (mb: number): string => G.size(mb);
