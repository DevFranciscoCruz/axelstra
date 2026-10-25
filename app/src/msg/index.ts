import { dict } from "../i18n";
import * as files from "./files";
import * as kanban from "./kanban";
import * as vcs from "./vcs";
import * as containers from "./containers";
import * as hosts from "./hosts";
import * as agents from "./agents";
import * as app from "./app";
import * as integrate from "./integrate";
import * as janitor from "./janitor";
import * as sources from "./sources";
import * as projectRemoval from "./projectRemoval";

/** one file per group of code prefixes, so parallel changes never touch the same file */
export const parts = {
  files,
  kanban,
  vcs,
  containers,
  hosts,
  agents,
  app,
  integrate,
  janitor,
  sources,
  projectRemoval,
};

const merge = (k: "pt" | "en" | "ptBR"): Record<string, string> =>
  Object.assign({}, ...Object.values(parts).map((p) => p[k]));

/** every message code -> its text in the UI language (`{name}` placeholders), read through the `locale()` signal */
export const MSG = dict<Record<string, string>>(merge("pt"), merge("en"), merge("ptBR"));

const MAX_PARAM = 300;
const clip = (v: string) => (v.length > MAX_PARAM ? v.slice(0, MAX_PARAM) + "…" : v);

/** `text` with `{name}` replaced by `params.name`, long values (tool output) clipped; an unknown name stays as written */
export const fill = (text: string, params?: Record<string, unknown>): string =>
  text.replace(/\{(\w+)\}/g, (m, k: string) => (params && k in params ? clip(String(params[k])) : m));

/** the code's text in the UI language, or undefined when the daemon sent a code this app does not know (an older app) */
export const msgText = (code: string | undefined, params?: Record<string, unknown>): string | undefined => {
  if (!code) return undefined;
  const t = (MSG as Record<string, string | undefined>)[code];
  return t === undefined ? undefined : fill(t, params);
};

/** a coded item (error, notice, event) as text: the translation when the code is known, else its English `fallback` */
export const coded = (
  code: string | undefined,
  params: Record<string, unknown> | undefined,
  fallback: string,
): string => msgText(code, params) ?? fallback;
