import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** joins class names, the later Tailwind utility winning over an earlier one */
export const cn = (...v: ClassValue[]) => twMerge(clsx(v));
