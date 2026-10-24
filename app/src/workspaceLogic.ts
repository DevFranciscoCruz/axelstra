import { WS } from "./strings.workspace";

export type WsSkipped = { label: string; reason: string; path?: string };
/** One kind of item of a workspace file compared with this machine (see `workspace.rs`). */
export type WsSection = {
  new: string[];
  present: string[];
  conflict: string[];
  skipped: WsSkipped[];
  local_only: number;
};
export type WorkspacePreview = {
  version: number;
  exported_at: string;
  projects: WsSection;
  automations: WsSection;
  settings: WsSection;
  cards: WsSection;
  risky_cards: number;
  /** folder map chosen so far: [path in the file, path here] */
  map: [string, string][];
  file: string;
};
export type WorkspaceOutcome = {
  projects_added: number;
  projects_updated: number;
  automations_added: number;
  automations_updated: number;
  settings_set: number;
  cards_written: number;
  cards_kept: number;
  files: string[];
};
export type WorkspaceMode = "merge" | "replace";
export type WorkspaceConflicts = "keep" | "overwrite";
export const KINDS = ["projects", "automations", "settings", "cards"] as const;

/** "2 novos · 1 já existe igual · 1 ignorado", or "nada" for an empty section. */
export const sectionLine = (s: WsSection): string => {
  const parts = [
    s.new.length && WS.nNew(s.new.length),
    s.present.length && WS.nPresent(s.present.length),
    s.conflict.length && WS.nConflict(s.conflict.length),
    s.skipped.length && WS.nSkipped(s.skipped.length),
    s.local_only && WS.localOnly(s.local_only),
  ].filter(Boolean);
  return parts.length ? parts.join(" · ") : WS.nothing;
};

export const reasonText = (r: string) => WS.reasons[r] ?? WS.reasonOther;

/** Replace needs the word typed; merge needs nothing. */
export const canImport = (mode: WorkspaceMode, typed: string, busy: boolean) =>
  !busy && (mode === "merge" || typed.trim().toLowerCase() === WS.replaceWord);

/** Skipped projects whose folder a map could fix: one entry per path of the file. */
export const mappable = (pv: WorkspacePreview): WsSkipped[] => {
  const seen = new Set<string>();
  return pv.projects.skipped.filter((s) => {
    if (s.reason !== "missing" || !s.path || seen.has(s.path)) return false;
    seen.add(s.path);
    return true;
  });
};

/** How many skipped items a section lists before «and N more». */
export const SKIPPED_SHOWN = 8;
