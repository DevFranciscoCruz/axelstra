/** Paths come back with either slash style; compare them in one normal form. The one `norm` for the whole app. */
export const norm = (p?: string | null) =>
  (p ?? "")
    .replace(/^\\\\\?\\/, "") // the Windows verbatim prefix, which the daemon drops too
    .replace(/\\/g, "/")
    .replace(/\/+$/, "")
    .toLowerCase();

/** last segment of a path with either slash style; trailing slashes ignored ("C:\a\b\\" -> "b") */
export const basename = (p: string | null | undefined): string =>
  (p ?? "")
    .replace(/[\\/]+$/, "")
    .split(/[\\/]/)
    .pop() ?? "";

/** a copy of `set` with `v` removed when it was in, added when it was not (checkbox-style toggles on a Set held in a signal) */
export const toggleIn = <T>(set: ReadonlySet<T>, v: T): Set<T> => {
  const next = new Set(set);
  if (!next.delete(v)) next.add(v);
  return next;
};

/** A remote path is `@<hostid>:<remote absolute path>`; the Tauri host adds and strips the prefix, the page keeps it opaque. */
const REMOTE_PATH = /^@([^:\s\\/]+):([\s\S]*)$/;
/** the host id of a remote path, null for a local one */
export const hostOf = (p: string | null | undefined): string | null => REMOTE_PATH.exec(p ?? "")?.[1] ?? null;
/** the path on the remote machine ("@lab:/home/me" -> "/home/me"); a local path comes back unchanged */
export const remotePathOf = (p: string): string => REMOTE_PATH.exec(p)?.[2] ?? p;
export const isRemote = (p: string | null | undefined): boolean => hostOf(p) !== null;
/** what the user reads: "lab:/home/me/proj" for a remote path (`hostName` replaces the id when known), the path itself for a local one */
export const displayPath = (p: string, hostName?: string): string => {
  const h = hostOf(p);
  return h === null ? p : `${hostName || h}:${remotePathOf(p)}`;
};
