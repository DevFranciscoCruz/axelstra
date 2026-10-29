import type { Diagnostics } from "./api";

/** "0.1.40+axelstra-daemon" -> [0,1,40]; null when there is no version in it (a daemon from before `build` existed). */
export function parseBuild(build: string | null | undefined): number[] | null {
  const m = /^(\d+)\.(\d+)\.(\d+)/.exec(build ?? "");
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

const cmp = (a: number[], b: number[]) => {
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  return 0;
};

/**
 * Is the daemon behind the app older than this app? The app already replaces a daemon below its PROTO or security
 * floor when it connects, so what is left is a daemon of an older release that still passes those: same contract, but
 * without the newest fixes. Only a daemon that answered counts; a missing one is the reconnect banner's business.
 */
export function daemonOlderThanApp(
  d: Pick<
    Diagnostics,
    | "app_version"
    | "app_proto"
    | "daemon_connected"
    | "daemon_proto"
    | "daemon_build"
    | "daemon_security"
    | "app_security_min"
  >,
): boolean {
  if (!d.daemon_connected) return false;
  if (d.daemon_proto != null && d.daemon_proto < d.app_proto) return true;
  if (d.daemon_security != null && d.daemon_security < d.app_security_min) return true;
  const daemon = parseBuild(d.daemon_build);
  const app = parseBuild(d.app_version);
  if (!app) return false;
  if (!daemon) return true; // a daemon that reports no build predates the field
  return cmp(daemon, app) < 0;
}

/** A 7-character commit, or null when it is missing or the build says it does not know it. */
export function shortCommit(c: string | null | undefined): string | null {
  const s = (c ?? "").trim();
  return s && s !== "unknown" ? s.slice(0, 7) : null;
}

export type BuildId = { version: string | null; commit: string | null };
export type StaleInfo = { show: boolean; reason: "busy" | "dev" | null; app: BuildId; daemon: BuildId };

/**
 * What the version-skew notice shows. It appears when the host kept an older daemon on purpose (`daemon_offer`, it
 * decided Offer at connect) or the daemon is of an older release than the app. The daemon's version falls back to the
 * one inside `daemon_build` for a daemon that does not report `daemon_version`.
 */
export function staleInfo(d: Diagnostics): StaleInfo {
  const offer = d.daemon_offer ?? null;
  const fromBuild = parseBuild(d.daemon_build);
  return {
    show: offer != null || daemonOlderThanApp(d),
    reason: offer?.reason ?? null,
    app: { version: d.app_version || null, commit: shortCommit(d.app_commit) },
    daemon: {
      version: d.daemon_version || (fromBuild ? fromBuild.join(".") : null),
      commit: shortCommit(d.daemon_commit),
    },
  };
}

/** "0.1.41 (abc1234)"; a missing version is `unknown`, a missing commit is left out. */
export const buildLabel = (b: BuildId, unknown: string) => (b.version ?? unknown) + (b.commit ? ` (${b.commit})` : "");
