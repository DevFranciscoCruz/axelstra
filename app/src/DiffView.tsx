import { Loading } from "./Loading";
import { For, Show, createEffect, createResource, createSignal } from "solid-js";
import { Button, Textarea } from "./ui";
import { Icon } from "./Icon";
import { PAGE, capList } from "./listing";
import { gitDiff, writeSession } from "./api";
import { P } from "./strings.panels";
import { activeSession, baseRef, changes, openDiff, selectedWorktree, sessionsOf, activate, setOpenDiff } from "./store";

type Row = { kind: "hunk" | "add" | "del" | "ctx" | "note"; old?: number; neu?: number; text: string };

const MAX_ROWS = 4000;

export function parseDiff(diff: string): Row[] {
  const rows: Row[] = [];
  let o = 0;
  let n = 0;
  let inHunk = false;
  for (const line of diff.split("\n")) {
    const h = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/.exec(line);
    if (h) {
      o = +h[1];
      n = +h[2];
      inHunk = true;
      rows.push({ kind: "hunk", text: line });
    } else if (!inHunk) {
      continue; // file header lines (diff --git, index, ---, +++)
    } else if (line.startsWith("+")) rows.push({ kind: "add", neu: n++, text: line.slice(1) });
    else if (line.startsWith("-")) rows.push({ kind: "del", old: o++, text: line.slice(1) });
    else if (line.startsWith(" ")) rows.push({ kind: "ctx", old: o++, neu: n++, text: line.slice(1) });
    else if (line.startsWith("\\") || line.startsWith("…")) rows.push({ kind: "note", text: line });
    if (rows.length >= MAX_ROWS) {
      rows.push({ kind: "note", text: P.diff.truncated(MAX_ROWS) });
      break;
    }
  }
  return rows;
}

export function DiffView() {
  // refetch when the file or its line counts change (the agent is still editing)
  const key = () => {
    const f = changes()?.files.find((x) => x.path === openDiff());
    return { wt: selectedWorktree(), path: openDiff(), sig: f ? `${f.added}/${f.removed}/${f.status}` : "", base: baseRef() };
  };
  const [text, { refetch }] = createResource(key, async (k) =>
    k.wt && k.path ? gitDiff(k.wt, k.path, k.base) : "",
  );
  createEffect(() => key() && void 0);
  const rows = () => parseDiff(text() ?? "");
  // a very long diff draws a page of rows at a time
  const [limit, setLimit] = createSignal(PAGE);
  createEffect(() => {
    key();
    setLimit(PAGE);
  });
  const page = () => capList(rows(), limit());

  // comment on a line and send it to a running agent of this worktree
  const [commenting, setCommenting] = createSignal<number | null>(null);
  const [note, setNote] = createSignal("");
  const [sendMsg, setSendMsg] = createSignal("");
  const sendFeedback = async (r: Row) => {
    const agents = sessionsOf(selectedWorktree()).filter((s) => s.running && s.meta?.kind === "agent");
    const target = agents.find((s) => s.id === activeSession()) ?? agents[0];
    if (!target) {
      setSendMsg(P.diff.noAgent);
      return;
    }
    const line = r.neu ?? r.old;
    const code = r.text.trim().replace(/\s+/g, " ").slice(0, 200);
    const msg = P.diff.review(openDiff() ?? "", line, code, note().trim().replace(/\s+/g, " "));
    try {
      await writeSession(target.id, msg + "\r");
      setCommenting(null);
      setOpenDiff(null);
      activate(target.id, "user");
    } catch (e) {
      setSendMsg(String(e));
    }
  };
  // one tab stop for all the comment buttons; the arrows move between them
  const [rov, setRov] = createSignal<number | null>(null);
  const firstBtn = () => page().shown.findIndex((r) => r.kind !== "hunk" && r.kind !== "note");
  const rove = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const t = e.target as HTMLElement;
    if (!t.classList?.contains("cbtn")) return;
    const all = [...(e.currentTarget as HTMLElement).querySelectorAll<HTMLElement>(".cbtn")];
    const next = all[all.indexOf(t) + (e.key === "ArrowDown" ? 1 : -1)];
    if (!next) return;
    e.preventDefault();
    next.focus();
  };
  const file = () => changes()?.files.find((x) => x.path === openDiff());

  return (
    <div class="diff">
      <div class="diff-head">
        <span class="mono">{openDiff()}</span>
        <Show when={file()}>
          {(f) => (
            <span class="stat">
              <span class="add">+{f().added}</span> <span class="del">−{f().removed}</span>
            </span>
          )}
        </Show>
        <Button variant="icon" size="icon" title={P.diff.reload} aria-label={P.diff.reload} onClick={() => void refetch()}>
          <Icon name="refresh" />
        </Button>
      </div>
      <div class="diff-body">
        <Show when={!text.loading} fallback={<Loading onRetry={() => void refetch()} />}>
          <Show when={rows().length > 0} fallback={<div class="placeholder">{P.diff.noText}</div>}>
            <table aria-label={P.diff.table} onKeyDown={rove}>
              <tbody>
                <For each={page().shown}>
                  {(r, i) => (
                    <>
                      <tr class={`row ${r.kind}`}>
                        <td class="ln">{r.old ?? ""}</td>
                        <td class="ln">
                          <Show when={r.kind !== "hunk" && r.kind !== "note"} fallback={r.neu ?? ""}>
                            <Button variant="icon" size="bare" class="cbtn px-0.5 text-xs hover:bg-transparent" title={P.diff.commentTip} aria-label={P.diff.commentLine((r.neu ?? r.old) as number)} tabindex={i() === (rov() ?? firstBtn()) ? 0 : -1} onFocus={() => setRov(i())} onClick={() => { setNote(""); setCommenting(commenting() === i() ? null : i()); }}>
                              ＋
                            </Button>
                            <span class="lnum">{r.neu ?? ""}</span>
                          </Show>
                        </td>
                        <td class="code">
                          {r.kind === "add" ? "+" : r.kind === "del" ? "−" : " "}
                          {r.text}
                        </td>
                      </tr>
                      <Show when={commenting() === i()}>
                        <tr class="cmt">
                          <td colspan="3">
                            <Textarea
                              placeholder={P.diff.placeholder}
                              value={note()}
                              onInput={(e) => setNote(e.currentTarget.value)}
                              ref={(el) => queueMicrotask(() => el.focus())}
                            />
                            <div class="cmt-actions">
                              <Button variant="default" size="sm" disabled={!note().trim()} onClick={() => void sendFeedback(r)}>
                                {P.diff.send}
                              </Button>
                              <Button size="sm" onClick={() => setCommenting(null)}>{P.diff.cancel}</Button>
                              <span class="cmt-msg" role="status" aria-live="polite">{sendMsg()}</span>
                            </div>
                          </td>
                        </tr>
                      </Show>
                    </>
                  )}
                </For>
              </tbody>
            </table>
            <Show when={page().hidden > 0}>
              <Button variant="outline" size="sm" class="show-more" onClick={() => setLimit(limit() + PAGE)}>
                {P.diff.more(page().hidden)}
              </Button>
            </Show>
          </Show>
        </Show>
      </div>
    </div>
  );
}
