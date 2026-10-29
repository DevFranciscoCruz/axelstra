import { createSignal } from "solid-js";
import { readString, writeString } from "./persist";
import { L } from "./strings.logic";

/** The pages of the settings screen, in the order of the side list, with the group each one sits under. */
export const SECTIONS = [
  { id: "general", group: "general", icon: "gear" },
  { id: "language", group: "general", icon: "grid" },
  { id: "appearance", group: "general", icon: "moon" },
  { id: "notifications", group: "general", icon: "bell" },
  { id: "memory", group: "performance", icon: "memory" },
  { id: "hibernate", group: "performance", icon: "clock" },
  { id: "build", group: "performance", icon: "cpu" },
  { id: "budgets", group: "performance", icon: "bolt" },
  { id: "connections", group: "integrations", icon: "plug" },
  { id: "workspace", group: "integrations", icon: "folder" },
  { id: "hooks", group: "integrations", icon: "branch" },
  { id: "maintenance", group: "maintenance", icon: "trash" },
  { id: "diagnostics", group: "maintenance", icon: "logs" },
  { id: "about", group: "maintenance", icon: "sparkle" },
] as const;

export type SectionId = (typeof SECTIONS)[number]["id"];
export type GroupId = (typeof SECTIONS)[number]["group"];
export const GROUPS: GroupId[] = ["general", "performance", "integrations", "maintenance"];

/** What the daemon reports: an older one has no `hibernate` or `build` settings, and then those pages do not exist. */
export type Caps = { hibernate: boolean; build: boolean };

export const SECTION_KEY = "axelstra.settings.section";
const FIRST: SectionId = "general";

export const isSectionId = (v: unknown): v is SectionId => SECTIONS.some((s) => s.id === v);

/** The page shown last time (a stored value that is not a page, or no storage, gives the first). */
export const loadSection = (read: (k: string) => string | null = readString): SectionId => {
  const v = read(SECTION_KEY);
  return isSectionId(v) ? v : FIRST;
};

const [section, setSectionRaw] = createSignal<SectionId>(loadSection());
export { section };
/** Shows a page of the settings and remembers it. */
export const setSection = (id: SectionId) => {
  setSectionRaw(id);
  writeString(SECTION_KEY, id);
};

export const available = (id: SectionId, caps: Caps): boolean =>
  id === "hibernate" ? caps.hibernate : id === "build" ? caps.build : true;

/** lower case without accents, so "memoria" finds "Memória" */
export const fold = (s: string): string => s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();

export type Item = { id: SectionId; group: GroupId; icon: string; title: string; sub: string };

/** The pages that exist for `caps` and match every word of `query` (in the title, the subtitle or the keywords). */
export const visibleItems = (caps: Caps, query: string, text = L.settingsNav): Item[] => {
  const words = fold(query).split(/\s+/).filter(Boolean);
  return SECTIONS.filter((s) => available(s.id, caps)).flatMap((s) => {
    const t = text.items[s.id];
    const hay = fold(`${t.title} ${t.sub} ${t.kw}`);
    return words.every((w) => hay.includes(w)) ? [{ ...s, title: t.title, sub: t.sub }] : [];
  });
};

/** The page to show: the chosen one when it exists, else the first that does. */
export const effectiveSection = (id: SectionId, caps: Caps): SectionId =>
  available(id, caps) ? id : (SECTIONS.find((s) => available(s.id, caps))?.id ?? FIRST);
