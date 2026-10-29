import { type Widen, count, fmtFixed, fmtNumber } from "./i18n";
import type { ptG } from "./strings.memory";

/** English texts of the resource governor (status bar chip, memory panel, Settings section, sidebar, notices). */
const size = (mb: number) => (mb >= 1024 ? `${fmtFixed(mb / 1024, 1)} GB` : `${fmtNumber(Math.round(mb))} MB`);

export const enG: Widen<typeof ptG> = {
  level: { normal: "normal", high: "high", critical: "critical" },
  size,
  chip: {
    text: (free: string) => `${free} free`,
    label: (level: string, free: string) => `Memory pressure ${level}: ${free} free. Opens the memory panel.`,
    pausedLabel: (n: number) => `${count(n, "item paused", "items paused")}. Opens the memory panel.`,
  },
  panel: {
    title: "Memory",
    summary: (level: string, free: string, total: string) => `Pressure ${level} · ${free} free of ${total}`,
    top: "Sessions using the most memory",
    topNone: "No per-session memory data.",
    rss: (s: string) => `${s} in use`,
    session: (id: number) => `Session ${id}`,
    pausedHead: "Paused",
    pausedNone: "Nothing is paused.",
    pausedMemory: "low memory",
    pausedManual: "manual",
    build: (name: string) => `build ${name}`,
    resume: "Resume",
    resumeOne: (who: string) => `Resume ${who}`,
    resumeAll: "Resume all",
    configure: "Memory settings",
    oldDaemon: "The running daemon has no resource governor. Restart Axelstra to use it.",
  },
  dot: {
    memory: "Paused: low memory",
    manual: "Paused (manual)",
    since: (what: string, ago: string) => `${what} · ${ago}`,
  },
  menu: {
    pause: "Pause",
    resume: "Resume",
  },
  sidebar: {
    rss: (s: string) => `Memory: ${s}`,
    pauseLabel: (who: string) => `Pause ${who}`,
    resumeLabel: (who: string) => `Resume ${who}`,
  },
  notify: {
    level: (level: "normal" | "high" | "critical", free: number): string =>
      level === "normal"
        ? "Memory is back to normal."
        : level === "high"
          ? `High memory pressure: ${size(free)} free.`
          : `Critical memory pressure: ${size(free)} free.`,
    paused: (n: number) => `${count(n, "session paused", "sessions paused")} for lack of memory.`,
    resumed: (n: number) => `${count(n, "session resumed", "sessions resumed")}: memory recovered.`,
  },
  settings: {
    title: "Resource governor",
    note: "Watches the machine's free memory. When it runs low it warns, delays the start of new agents and can pause the biggest consumers so the machine does not run out. You resume whenever you like, or they resume by themselves when memory recovers.",
    enabled: "Enable the governor",
    admission: "Wait for memory before starting agents and builds",
    autoPause: "Pause sessions automatically at critical memory",
    warnOnly: "Warn only: never pause or delay anything",
    warnOnlyHint: "Handy to see what the governor would do without it touching anything.",
    thresholds: "Free-memory thresholds",
    thresholdsNote:
      "The level rises when free memory drops below the threshold in percent or in MB. Critical must be lower than high.",
    highPct: "High: free below (%)",
    critPct: "Critical: free below (%)",
    highMb: "High: free below (MB)",
    critMb: "Critical: free below (MB)",
    save: "Save thresholds",
    saved: "Memory thresholds saved.",
    notNumber: "Enter whole numbers in every threshold field.",
    problem: {
      pct_range: "Percentages must be between 1 and 99.",
      mb_range: "Values in MB must be greater than zero.",
      pct_order: "The critical (%) threshold must be lower than the high one.",
      mb_order: "The critical (MB) threshold must be lower than the high one.",
    },
  },
};
