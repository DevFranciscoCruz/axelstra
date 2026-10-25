import type { Diagnostics, HostInfoReport } from "./api";
import { fmtDate } from "./i18n";
import { daemonOlderThanApp, parseBuild } from "./skewLogic";
import { L } from "./strings.logic";

/** How the running daemon relates to this app. */
export type Skew = "inSync" | "older" | "newer" | "differs" | "noDaemon";

export type AboutInfo = {
  appVersion: string;
  appCommit: string | null;
  appBuilt: string | null;
  daemonConnected: boolean;
  daemonVersion: string | null;
  daemonCommit: string | null;
  daemonBuilt: string | null;
  proto: number | null;
  buildId: string | null;
  os: string | null;
  webview: string | null;
  skew: Skew;
};

/** A commit or date the build could not tell is "unknown" (or absent on an old build): both mean not known. */
const known = (v: string | null | undefined): string | null => (v && v !== "unknown" ? v : null);

const cmp = (a: number[], b: number[]) => {
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  return 0;
};

/** The About model from what the host (`diagnostics`) and the daemon (`host.info`) reported; every part may be missing. */
export function aboutInfo(d: Diagnostics, h: HostInfoReport | null): AboutInfo {
  const daemonNums = parseBuild(h?.version) ?? parseBuild(d.daemon_build);
  const appNums = parseBuild(d.app_version);
  const appCommit = known(d.app_commit);
  const daemonCommit = known(h?.commit);
  let skew: Skew = "inSync";
  if (!d.daemon_connected) skew = "noDaemon";
  else if (daemonOlderThanApp(d)) skew = "older";
  else if (daemonNums && appNums && cmp(daemonNums, appNums) > 0) skew = "newer";
  else if (appCommit && daemonCommit && appCommit !== daemonCommit) skew = "differs";
  return {
    appVersion: d.app_version,
    appCommit,
    appBuilt: known(d.app_built),
    daemonConnected: d.daemon_connected,
    daemonVersion: daemonNums ? daemonNums.join(".") : null,
    daemonCommit,
    daemonBuilt: known(h?.built),
    proto: d.daemon_proto,
    buildId: d.daemon_build,
    os: d.os ?? null,
    webview: d.webview ?? null,
    skew,
  };
}

/** "2026-10-25" (a UTC build date) in the language of the UI; anything else is kept as it is. */
export function fmtBuilt(iso: string | null): string {
  if (!iso) return L.about.unknown;
  const t = /^\d{4}-\d{2}-\d{2}$/.test(iso) ? Date.parse(`${iso}T00:00:00Z`) : NaN;
  return Number.isNaN(t) ? iso : fmtDate(t, { dateStyle: "medium", timeZone: "UTC" });
}

const or = (v: string | number | null) => (v == null ? L.about.unknown : String(v));

/** The warning for a daemon that is not the app's own version; null when there is nothing to warn about. */
export function skewWarning(a: AboutInfo): string | null {
  const daemon = `v${or(a.daemonVersion)}`;
  const app = `v${a.appVersion}`;
  switch (a.skew) {
    case "older":
      return L.about.older(daemon, app);
    case "newer":
      return L.about.newer(daemon, app);
    case "differs":
      return L.about.differs;
    case "noDaemon":
      return L.about.noDaemon;
    default:
      return null;
  }
}

/** Does restarting the daemon fix the warning? (an older daemon or another commit: yes; a newer daemon: update the app.) */
export const restartHelps = (a: AboutInfo) => a.skew === "older" || a.skew === "differs";

/** The rows of the app and of the daemon, in the order they are shown and copied. */
export function aboutRows(a: AboutInfo): { app: [string, string][]; daemon: [string, string][] } {
  const A = L.about;
  return {
    app: [
      [A.version, a.appVersion],
      [A.commit, or(a.appCommit)],
      [A.built, fmtBuilt(a.appBuilt)],
      [A.os, or(a.os)],
      [A.webview, or(a.webview)],
    ],
    daemon: a.daemonConnected
      ? [
          [A.version, or(a.daemonVersion)],
          [A.commit, or(a.daemonCommit)],
          [A.built, fmtBuilt(a.daemonBuilt)],
          [A.proto, or(a.proto)],
          [A.buildId, or(a.buildId)],
        ]
      : [[A.daemon, A.offline]],
  };
}

/** Plain text for the clipboard (a bug report). */
export function aboutText(a: AboutInfo): string {
  const { app, daemon } = aboutRows(a);
  const block = (title: string, rows: [string, string][]) =>
    [title, ...rows.map(([k, v]) => `  ${k}: ${v}`)].join("\n");
  return [block(`Axelstra · ${L.about.app}`, app), block(L.about.daemon, daemon)].join("\n");
}

/** "v0.1.0 · a1b2c3d" for the status bar; just the version when the commit is not known. */
export const chipText = (a: Pick<AboutInfo, "appVersion" | "appCommit">) =>
  a.appCommit ? `v${a.appVersion} · ${a.appCommit}` : `v${a.appVersion}`;
