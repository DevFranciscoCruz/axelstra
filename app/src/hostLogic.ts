import type { HostInfo, HostState } from "./api";

/** Why a host spec cannot be used. Client-side only (UX): the Tauri host validates again. */
export type SpecProblem = "empty" | "space" | "dash" | "scheme" | "distro" | "host" | "user" | "port";

const NAME = /^[A-Za-z0-9._-]+$/;
const IPV6 = /^\[[0-9A-Fa-f:.]+\]$/;

/** null when `spec` is `wsl:<distro>` or `ssh:[user@]host[:port]` under the rules of docs/remote-daemon.md. */
export function specProblem(raw: string): SpecProblem | null {
  const spec = raw.trim();
  if (!spec) return "empty";
  if (/\s/.test(spec) || [...spec].some((c) => c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127)) return "space";
  const i = spec.indexOf(":");
  const scheme = i < 0 ? "" : spec.slice(0, i);
  const rest = i < 0 ? "" : spec.slice(i + 1);
  if (scheme !== "wsl" && scheme !== "ssh") return "scheme";
  if (scheme === "wsl") {
    if (rest.startsWith("-")) return "dash";
    return NAME.test(rest) ? null : "distro";
  }
  let user: string | null = null;
  let hostPort = rest;
  const at = rest.indexOf("@");
  if (at >= 0) {
    user = rest.slice(0, at);
    hostPort = rest.slice(at + 1);
  }
  let host = hostPort;
  let port: string | null = null;
  if (hostPort.startsWith("[")) {
    const end = hostPort.indexOf("]");
    if (end < 0) return "host";
    host = hostPort.slice(0, end + 1);
    const after = hostPort.slice(end + 1);
    if (after) {
      if (!after.startsWith(":")) return "host";
      port = after.slice(1);
    }
  } else {
    const c = hostPort.indexOf(":");
    if (c >= 0) {
      host = hostPort.slice(0, c);
      port = hostPort.slice(c + 1);
    }
  }
  if (user !== null && user.startsWith("-")) return "dash";
  if (host.startsWith("-") || port?.startsWith("-")) return "dash";
  if (user !== null && !NAME.test(user)) return "user";
  if (!(NAME.test(host) || IPV6.test(host))) return "host";
  if (port !== null && !(/^\d{1,5}$/.test(port) && Number(port) >= 1 && Number(port) <= 65535)) return "port";
  return null;
}

/** apply a `host-state` event to the list; an unknown id (a host added elsewhere) leaves it as it was */
export function applyHostState(
  list: HostInfo[],
  ev: { id: string; state: HostState; error?: string | null },
): HostInfo[] {
  if (!list.some((h) => h.id === ev.id)) return list;
  return list.map((h) =>
    h.id === ev.id ? { ...h, state: ev.state, error: ev.state === "ok" ? undefined : (ev.error ?? undefined) } : h,
  );
}

/** states from which "Voltar a ligar" makes sense */
export const canReconnect = (s: HostState | undefined): boolean => s === "lost" || s === "auth" || s === "idle";

/** how the remote error text reads to the user */
export type RemoteFailure = "auth" | "no-axelstra" | "no-daemon" | "unreachable" | "other";
export function classifyRemoteError(msg: string): RemoteFailure {
  const m = msg.toLowerCase();
  if (/permission denied|publickey|authentication|host key verification|no more authentication|sem permiss/.test(m))
    return "auth";
  if (/daemon not reachable|not reachable on this machine|start it there|daemon.*not running/.test(m))
    return "no-daemon";
  if (/command not found|not found|no such file|unrecognized subcommand|não encontrado/.test(m)) return "no-axelstra";
  if (
    /timed out|timeout|could not resolve|unreachable|connection (refused|reset|closed)|no route|name or service|broken pipe|eof/.test(
      m,
    )
  )
    return "unreachable";
  return "other";
}

export type Crumb = { label: string; path: string };
/** Breadcrumb of a remote absolute path: "/" then each segment ("C:\" first on a Windows remote). */
export function breadcrumbs(path: string): Crumb[] {
  if (!path) return [];
  const win = /^[A-Za-z]:[\\/]?/.exec(path);
  const sep = win || path.includes("\\") ? "\\" : "/";
  const parts = path.split(/[\\/]+/).filter(Boolean);
  const out: Crumb[] = [];
  let acc: string;
  if (win) {
    acc = `${parts.shift()}${sep}`;
    out.push({ label: acc, path: acc });
  } else {
    acc = sep;
    out.push({ label: sep, path: sep });
  }
  for (const p of parts) {
    acc = acc.endsWith(sep) ? `${acc}${p}` : `${acc}${sep}${p}`;
    out.push({ label: p, path: acc });
  }
  return out;
}

/** `dir` + `name` with the remote's separator */
export const joinRemote = (dir: string, name: string, sep = "/"): string =>
  dir.endsWith(sep) ? `${dir}${name}` : `${dir}${sep}${name}`;

/** an absolute remote path: unix ("/x") or Windows ("C:\x") */
export const isAbsRemote = (p: string): boolean => /^(\/|[A-Za-z]:[\\/])/.test(p.trim());
