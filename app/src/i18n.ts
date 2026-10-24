import { createSignal } from "solid-js";
import { readString, writeString } from "./persist";

/**
 * Typed dictionaries per locale. pt-PT is the source: its shape is the type every other locale must satisfy, so a key
 * missing from `en` is a compile error (`dict` below), and `i18n.test.ts` repeats the check at run time. A dictionary is
 * read through `S.some.key` as before; the read goes through the `locale` signal, so JSX and effects update when the
 * language changes. Values computed once at module load keep the language of that moment: build them in a function.
 */
export const LOCALES = ["pt-PT", "en", "pt-BR"] as const;
export type Locale = (typeof LOCALES)[number];
export const DEFAULT_LOCALE: Locale = "pt-PT";
/** What the selector shows: each language in its own name. */
export const LOCALE_NAMES: Record<Locale, string> = {
  "pt-PT": "Português (Portugal)",
  en: "English",
  "pt-BR": "Português (Brasil)",
};

const KEY = "axelstra.locale";
export const isLocale = (v: unknown): v is Locale =>
  typeof v === "string" && (LOCALES as readonly string[]).includes(v);

const stored = (): Locale => {
  const v = readString(KEY);
  return isLocale(v) ? v : DEFAULT_LOCALE;
};

export const [locale, setLocaleSig] = createSignal<Locale>(stored());

const apply = (l: Locale) => {
  try {
    document.documentElement.lang = l;
  } catch {
    /* no document (tests) */
  }
};

/** Applies live (every dictionary read follows the signal) and is kept in localStorage and, through the mirror, in the daemon's settings. */
export const setLocale = (l: Locale) => {
  setLocaleSig(l);
  writeString(KEY, l);
  apply(l);
};

/** Call once the UI state was restored from the daemon (a new WebView profile gets its language back). */
export const initLocale = () => {
  const l = stored();
  setLocaleSig(l);
  apply(l);
};

// ---- typed dictionaries ----

/** The shape `T` must be given in another locale: same keys, every text a string, every function the same parameters. */
export type Widen<T> = T extends string
  ? string
  : T extends (...a: infer A) => string
    ? (...a: A) => string
    : T extends readonly (infer U)[]
      ? Widen<U>[]
      : T extends object
        ? { [K in keyof T]: Widen<T[K]> }
        : T;
export type DeepPartial<T> = T extends (...a: never[]) => unknown
  ? T
  : T extends readonly unknown[]
    ? T
    : T extends object
      ? { [K in keyof T]?: DeepPartial<T[K]> }
      : T;

type Layers = () => unknown[];
const isPlain = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);

const lookup = (layers: unknown[], path: string[]): unknown => {
  for (const layer of layers) {
    let o: unknown = layer;
    for (const k of path) o = isPlain(o) ? o[k] : undefined;
    if (o !== undefined) return o;
  }
  return undefined;
};

const node = <T extends object>(layers: Layers, path: string[], shape: T): T =>
  new Proxy(shape, {
    get(target, key, receiver) {
      if (typeof key === "symbol" || !(key in target)) return Reflect.get(target, key, receiver);
      const p = [...path, key];
      const v = lookup(layers(), p);
      return isPlain(v) ? node(layers, p, (target as Record<string, object>)[key]) : v;
    },
  });

/**
 * `dict(pt, en)` is `pt` read in the current language. `ptBR` may be partial: a missing key falls back to pt-PT (the two
 * differ in a few words, not in every sentence). Every other locale must be complete, which `Widen` checks at compile time.
 */
export function dict<T extends object>(
  pt: T,
  en: Widen<T>,
  ptBR: DeepPartial<Widen<T>> = {} as DeepPartial<Widen<T>>,
): T {
  const by: Record<Locale, unknown> = { "pt-PT": pt, en, "pt-BR": ptBR };
  // eslint-disable-next-line solid/reactivity -- read lazily on every access, by design: that is what makes the dictionary live
  return node(() => (locale() === "pt-PT" ? [pt] : [by[locale()], pt]), [], pt);
}

/** every path of leaf texts in a dictionary, for the completeness test */
export const leafPaths = (o: unknown, prefix = ""): string[] =>
  isPlain(o) ? Object.entries(o).flatMap(([k, v]) => leafPaths(v, prefix ? `${prefix}.${k}` : k)) : [prefix];

// ---- plurals and formatting with the locale ----

const rules = new Map<Locale, Intl.PluralRules>();
/** `one` for exactly 1 in pt-PT and English (pt-PT says "0 worktrees"; pt-BR counts 0 as singular). Returns the word only. */
export const plural = (n: number, one: string, other: string): string => {
  const l = locale();
  let r = rules.get(l);
  if (!r) rules.set(l, (r = new Intl.PluralRules(l)));
  return r.select(n) === "one" ? one : other;
};
/** "1 worktree" / "3 worktrees" */
export const count = (n: number, one: string, other: string): string => `${fmtNumber(n)} ${plural(n, one, other)}`;

export const fmtNumber = (n: number, o?: Intl.NumberFormatOptions): string =>
  new Intl.NumberFormat(locale(), o).format(n);
/** a number with exactly `digits` decimals, in the decimal mark of the language */
export const fmtFixed = (n: number, digits: number): string =>
  fmtNumber(n, { minimumFractionDigits: digits, maximumFractionDigits: digits });
export const fmtDate = (
  d: Date | number,
  o: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" },
): string => new Intl.DateTimeFormat(locale(), o).format(d);
/** "há 5 minutos" / "5 minutes ago"; `unit` as Intl.RelativeTimeFormat wants it */
export const fmtRelative = (value: number, unit: Intl.RelativeTimeFormatUnit): string =>
  new Intl.RelativeTimeFormat(locale(), { numeric: "auto" }).format(value, unit);

// ---- tables whose texts must be read when used, not when the module loads ----

/** `{ a: () => S.x }` as an object whose `a` reads `S.x` each time (a table kept at module level still follows the language). */
export const lazy = <T extends Record<string, () => string>>(fns: T): { [K in keyof T]: string } =>
  Object.defineProperties(
    {},
    Object.fromEntries(Object.entries(fns).map(([k, f]) => [k, { get: f, enumerable: true }])),
  ) as { [K in keyof T]: string };

/** `[icon, text]` where the text is read when used */
export const lazyPair = (icon: string, text: () => string): [string, string] =>
  Object.defineProperty([icon, ""], 1, { get: text, enumerable: true }) as [string, string];
