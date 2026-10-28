import { type Widen, count, fmtFixed, fmtNumber } from "./i18n";
import type { ptPF } from "./strings.perf";

/** English texts of the Performance section of Settings. */
export const enPF: Widen<typeof ptPF> = {
  title: "Performance",
  intro:
    "Worktrees create and delete many small files, and the antivirus and the system indexer read each one. Excluding the .axelstra-wt folders from those readers speeds up git, installs and builds.",
  autoIndexer: "Without doing anything: Axelstra already marks these folders so the system does not index them.",
  tradeoff:
    "Trade-off: a folder excluded from Defender stops being scanned in real time. We only recommend this for Axelstra's own .axelstra-wt folders, never for your projects.",
  loading: "Reading the state…",
  loadingSlow: "Reading is taking a while. Refresh in a moment.",
  unavailable: "Could not read the state of the exclusions.",
  macos:
    "On macOS Axelstra puts a .metadata_never_index file in .axelstra-wt so Spotlight skips it. Nothing else to set up.",
  linux:
    "On Linux Axelstra changes nothing. If tracker or baloo indexes the worktrees, exclude the .axelstra-wt folder in the indexer settings.",
  noFolders: "There are no .axelstra-wt folders yet: they appear when you create the first worktree.",
  foldersLabel: "Worktree folders",
  folderMissing: "the folder does not exist yet",
  indexable: { yes: "indexable by the system", no: "kept out of the system index", unknown: "system index unknown" },
  excluded: {
    yes: "excluded from Defender",
    recorded: "excluded from Defender (applied by Axelstra)",
    no: "not excluded from Defender",
    unknown: "Defender exclusion unknown",
    na: "no Defender",
  },
  devDrive: "Dev Drive (ReFS): already scanned lightly, no exclusion needed.",
  fsLine: (fs: string, drive: string) => `${drive}: ${fs}`,
  sharedBy: (names: string) => `Projects: ${names}`,
  defenderState: {
    active: "Defender is the active antivirus.",
    inactive: "Defender is not the active antivirus: another product (or none) scans files.",
    unavailable: "Could not query Defender.",
    na: "No Defender on this system.",
  },
  realtimeOn: "Real-time scanning is on.",
  realtimeOff: "Real-time scanning is off.",
  tamperOn: "Tamper Protection is on: Defender may refuse the exclusion.",
  unreadable:
    "Without administrator rights Axelstra cannot read Defender's exclusions; it only knows the ones it applied itself.",
  consent: "I understand the trade-off and want to exclude the .axelstra-wt folders from Defender",
  uac: "Windows will ask for administrator permission (UAC). Nothing changes before you accept.",
  apply: "Apply…",
  revert: "Revert",
  revertNote: "Revert removes only the exclusions Axelstra applied, never yours.",
  running: "Waiting for the administrator prompt…",
  applied: (n: number, same: number) =>
    `Exclusion applied to ${count(n, "folder", "folders")}` +
    (same ? ` (${count(same, "was already excluded", "were already excluded")})` : "."),
  appliedNone: "Nothing to change: the folders were already excluded.",
  reverted: (n: number) => `Exclusion removed from ${count(n, "folder", "folders")}.`,
  revertedNone: "Nothing to revert.",
  failed: (detail: string) => `Could not finish: ${detail}`,
  diagTitle: "Antivirus and indexer load",
  diagNote: "A single reading of the processor and memory of the Defender and Windows indexer processes.",
  diagRefresh: "Refresh",
  diagNone: "None of these processes is running.",
  diagNa: "Only available on Windows.",
  diagProc: (name: string, cpu: number | null, rss: number | null) =>
    `${name}: ${cpu === null ? "—" : `${fmtFixed(cpu, 0)} % of one core`} · ${rss === null ? "—" : `${fmtNumber(Math.round(rss / 1048576))} MB`}`,
  hintTitle: "Faster worktrees?",
  hintSub:
    "Defender is scanning the worktrees folder in real time. See the options in Settings > Performance. Nothing changes without your click.",
  hintOpen: "See options",
  hintDismiss: "Do not show again",
  hintLabel: "Performance suggestion",
};
