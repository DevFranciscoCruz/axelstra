import { Loading } from "./Loading";
import { For, Show, createResource } from "solid-js";
import { gitShow } from "./api";
import { parseDiff } from "./DiffView";
import { openCommit, selectedWorktree } from "./store";

type FileDiff = { path: string; text: string };

/** Splits one `git show` patch into per-file chunks. */
function splitFiles(patch: string): FileDiff[] {
  const out: FileDiff[] = [];
  let cur: FileDiff | null = null;
  for (const line of patch.split("\n")) {
    const m = /^diff --git a\/(.*) b\/(.*)$/.exec(line);
    if (m) {
      cur = { path: m[2], text: "" };
      out.push(cur);
    }
    if (cur) cur.text += line + "\n";
  }
  return out;
}

export function CommitView() {
  const key = () => ({ wt: selectedWorktree(), sha: openCommit()?.sha });
  const [patch, { refetch }] = createResource(key, async (k) => (k.wt && k.sha ? gitShow(k.wt, k.sha) : ""));
  const files = () => splitFiles(patch() ?? "");

  return (
    <div class="diff">
      <div class="diff-head">
        <span class="mono">{openCommit()?.short}</span>
        <span>{openCommit()?.subject}</span>
      </div>
      <div class="diff-body">
        <Show when={!patch.loading} fallback={<Loading onRetry={() => void refetch()} />}>
          <For each={files()} fallback={<div class="placeholder">Commit sem alterações de texto</div>}>
            {(f) => {
              const rows = parseDiff(f.text);
              return (
                <>
                  <div class="cfile">{f.path}</div>
                  <Show
                    when={rows.length > 0}
                    fallback={<div class="empty">Sem diferenças de texto (binário ou ficheiro vazio)</div>}
                  >
                    <table>
                      <tbody>
                        <For each={rows}>
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
                </>
              );
            }}
          </For>
        </Show>
      </div>
    </div>
  );
}
