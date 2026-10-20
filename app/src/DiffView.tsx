import { For, Show, createEffect, createResource } from "solid-js";
import { gitDiff } from "./api";
import { baseRef, changes, openDiff, selectedWorktree } from "./store";

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
      rows.push({ kind: "note", text: `… mostrando só as primeiras ${MAX_ROWS} linhas` });
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
        <button class="icon" title="Recarregar" onClick={() => void refetch()}>
          ↻
        </button>
      </div>
      <div class="diff-body">
        <Show when={!text.loading} fallback={<div class="placeholder">A carregar…</div>}>
          <Show when={rows().length > 0} fallback={<div class="placeholder">Sem diferenças de texto para mostrar</div>}>
            <table>
              <tbody>
                <For each={rows()}>
                  {(r) => (
                    <tr class={`row ${r.kind}`}>
                      <td class="ln">{r.old ?? ""}</td>
                      <td class="ln">{r.neu ?? ""}</td>
                      <td class="code">
                        {r.kind === "add" ? "+" : r.kind === "del" ? "−" : " "}
                        {r.text}
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        </Show>
      </div>
    </div>
  );
}
