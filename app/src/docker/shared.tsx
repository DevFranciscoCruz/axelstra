import { Icon } from "../Icon";
import { setError, showNotice } from "../store";
import { P } from "../strings.panels";

export type DTab = "info" | "logs" | "terminal" | "files";
export const DTABS: [DTab, string][] = (["info", "logs", "terminal", "files"] as DTab[]).map((t) => [
  t,
  P.docker.tabs[t],
]);

/** The docker toolbar icons live in the shared Icon set; this keeps the 14px size of the lists. */
export const DIcon = (props: { name: string }) => <Icon name={props.name} size={14} />;

/** results go to the app toast, like everywhere else */
export const say = (text: string, err = false) => (err ? setError(text) : showNotice(text));
export const nameOf = (e: unknown) => String(e).replace(/^Error: /, "");

/** shell sessions started from the detail pane, per container id, so coming back to the tab finds the same shell */
export const shells = new Map<string, number>();
