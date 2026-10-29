import { type Widen, count } from "./i18n";
import type { ptH } from "./strings.hibernate";

/** English texts of session hibernation (Settings section, sidebar, dot tooltips, notices). */
export const enH: Widen<typeof ptH> = {
  settings: {
    title: "Session hibernation",
    note: "An agent that only waits (for CI, for a worker, for an answer) can be put to sleep: its process ends and the memory is free, but its row and its conversation stay. It wakes by itself when a message arrives, a worker finishes or you type into it.",
    enabled: "Hibernate idle sessions automatically",
    idle: "Hibernate after",
    idleMin: (n: number) => (n >= 60 && n % 60 === 0 ? count(n / 60, "hour", "hours") : count(n, "minute", "minutes")),
  },
  menu: { hibernate: "Hibernate", wake: "Wake" },
  hibernateLabel: (name: string) => `Hibernate ${name}`,
  wakeLabel: (name: string) => `Wake ${name}`,
  dot: {
    hibernated: "Hibernated: its process ended, it wakes when needed",
    waking: "Waking up",
    queued: "Waking up: waiting for a worker slot",
    since: (what: string, ago: string, mb: number) => `${what} (${ago}${mb > 0 ? `, ${mb} MB freed` : ""})`,
  },
  notice: {
    inputDropped: (name: string) => `${name} was hibernated: it woke up, but what you typed was lost. Type it again.`,
    wakeFailed: (name: string, why: string) => `${name} could not wake up: ${why}`,
  },
};
