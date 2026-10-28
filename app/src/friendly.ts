import { S } from "./strings";
import { L } from "./strings.logic";

/** the daemon and git speak English; the common failures map to a sentence (in the UI language) that says what to do */
const FRIENDLY: [RegExp, keyof typeof L.friendly][] = [
  [/os error 2|program not found|not recognized|cannot find the file/i, "notFound"],
  [/is already checked out|already exists/i, "exists"],
  [/not a git repository/i, "notRepo"],
  [/contains modified or untracked files|local changes would be overwritten/i, "dirty"],
  [
    /could not read from remote|authentication failed|permission denied \(publickey\)|could not resolve host/i,
    "remote",
  ],
  [/non-fast-forward|failed to push|rejected/i, "rejected"],
  [/worker limit reached/i, "workerLimit"],
  [/connection (refused|reset)|pipe.*(closed|broken)|os error 232|os error 109/i, "noDaemon"],
  [/sem resposta do daemon/i, "daemonSilent"],
  [/unknown method/i, "oldDaemon"],
  [/cannot connect to the docker daemon|docker daemon is not running|error during connect/i, "dockerDown"],
  [/no such container|no such image|no such network|no such volume/i, "dockerGone"],
  [/port is already allocated|address already in use/i, "portBusy"],
  [/gh auth login|not logged in|http 401|bad credentials|requires authentication/i, "ghAuth"],
  [/merge conflict|automatic merge failed|conflict \(/i, "conflicts"],
  [/nothing to commit|no changes added to commit/i, "nothingToCommit"],
  [/pathspec .* did not match|unknown revision|bad revision|not a valid object name|invalid reference/i, "badRef"],
  [/rate limit/i, "rateLimit"],
];

/** what a message from git, gh or docker usually looks like: English, with their prefixes */
const TOOL_ENGLISH =
  /^\s*(fatal|error|warning|hint|remote|gh|docker|git)\b[:\s]|error response from daemon|\bgit\b.*\b(failed|exited)\b|\bdocker\b.*\b(failed|exited)\b|\bgh\b.*\b(failed|exited)\b/i;

const clip = (m: string) => (m.length > 600 ? m.slice(0, 600) + "…" : m);

/** the message in the UI language when it is a known failure, with the original in brackets; an unknown message from git, gh or
 *  docker gets a line in the UI language in front, so it never shows raw English alone in a Portuguese UI; anything else is left as it is */
export const friendlyError = (m: string) => {
  const hit = FRIENDLY.find(([re]) => re.test(m));
  if (hit) return `${L.friendly[hit[1]]}\n(${clip(m)})`;
  if (TOOL_ENGLISH.test(m)) return `${S.toolFailed}\n(${clip(m)})`;
  return m;
};
