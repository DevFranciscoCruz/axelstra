import { createSignal } from "solid-js";
import type { ScanxFolder, ScanxJobResult, ScanxProc, ScanxStatus } from "./api";
import { msgText } from "./msg";
import { PF } from "./strings.perf";

/** Bumped by the one-time notice: the Performance section scrolls to itself and takes focus when it sees a new value. */
export const [perfFocus, setPerfFocus] = createSignal(0);

/** The one-time notice shows only while the daemon says it is pending and the user has not dismissed it. */
export const hintVisible = (s: ScanxStatus | null | undefined): boolean =>
  !!s && s.hint && !s.hint_dismissed && s.os === "windows" && s.defender.state === "active";

/** A folder row, one per distinct path (projects of the same parent share a folder). */
export type FolderRow = {
  path: string;
  /** project names that share the folder */
  names: string[];
  indexable: string;
  excluded: string;
  fs: string;
  devDrive: boolean;
  missing: boolean;
};

const excludedText = (f: ScanxFolder): string => {
  if (f.excluded === "yes") return f.source === "recorded" ? PF.excluded.recorded : PF.excluded.yes;
  if (f.excluded === "no") return PF.excluded.no;
  if (f.excluded === "unknown") return PF.excluded.unknown;
  return PF.excluded.na;
};
const indexableText = (v: boolean | null): string =>
  v === null ? PF.indexable.unknown : v ? PF.indexable.yes : PF.indexable.no;

export const folderRows = (s: ScanxStatus): FolderRow[] => {
  const by = new Map<string, FolderRow>();
  for (const f of s.folders) {
    const have = by.get(f.path);
    if (have) {
      if (!have.names.includes(f.name)) have.names.push(f.name);
      continue;
    }
    by.set(f.path, {
      path: f.path,
      names: [f.name],
      indexable: indexableText(f.indexable),
      excluded: excludedText(f),
      fs: f.fs && f.drive ? PF.fsLine(f.fs, f.drive) : "",
      devDrive: f.dev_drive,
      missing: !f.exists,
    });
  }
  return [...by.values()];
};

/** The lines about Defender itself; empty off Windows. */
export const defenderLines = (s: ScanxStatus): string[] => {
  const d = s.defender;
  if (s.os !== "windows" || d.state === "n/a") return [];
  const out: string[] = [PF.defenderState[d.state]];
  if (d.state === "active" && d.realtime !== null) out.push(d.realtime ? PF.realtimeOn : PF.realtimeOff);
  if (d.tamper) out.push(PF.tamperOn);
  if (!d.exclusions_readable) out.push(PF.unreadable);
  return out;
};

/** Neutral note for the systems without a Defender; null on Windows. */
export const platformNote = (s: ScanxStatus): string | null =>
  s.os === "macos" ? PF.macos : s.os === "linux" ? PF.linux : null;

export type Buttons = {
  showControls: boolean;
  consentDisabled: boolean;
  applyDisabled: boolean;
  showRevert: boolean;
  revertDisabled: boolean;
};
/** Which controls the panel offers: apply needs the consent checkbox and a free daemon; revert only when something was applied. */
export const buttons = (s: ScanxStatus, consent: boolean, busy: boolean): Buttons => {
  const running = busy || s.job.running !== null;
  const showControls = s.os === "windows" && s.defender.state !== "n/a" && s.folders.length > 0;
  return {
    showControls,
    consentDisabled: running,
    applyDisabled: running || !consent || !s.can_apply,
    showRevert: s.can_revert,
    revertDisabled: running,
  };
};

export type JobMessage = { kind: "ok" | "err"; text: string };
/** The outcome of the last apply/revert (a notice-only `hint` job says nothing). */
export const jobMessage = (r: ScanxJobResult | null | undefined): JobMessage | null => {
  if (!r || r.op === "hint") return null;
  if (!r.ok) {
    const text = msgText(r.error_code, r.error_params) ?? r.error ?? PF.unavailable;
    return { kind: "err", text: PF.failed(text) };
  }
  const n = r.changed?.length ?? 0;
  const same = r.unchanged?.length ?? 0;
  if (r.op === "revert") return { kind: "ok", text: n ? PF.reverted(n) : PF.revertedNone };
  return { kind: "ok", text: n ? PF.applied(n, same) : PF.appliedNone };
};

/** Readable text of a failed status query (coded when the daemon gave a code). */
export const statusError = (s: ScanxStatus): string | null =>
  s.error || s.error_code ? (msgText(s.error_code, s.error_params) ?? s.error ?? PF.unavailable) : null;

export const procLine = (p: ScanxProc): string => PF.diagProc(p.name, p.cpu, p.rss);
