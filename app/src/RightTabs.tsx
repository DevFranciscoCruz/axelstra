import { Show } from "solid-js";
import { changes, rightTab, setRightTab } from "./store";

/** Tab strip shared by every right-hand panel. */
export function RightTabs() {
  const n = () => changes()?.files.length ?? 0;
  const tab = (id: "files" | "changes" | "pulls", label: string) => (
    <span class="rtab" classList={{ active: rightTab() === id }} onClick={() => setRightTab(id)}>
      {label}
      <Show when={id === "changes" && n()}> {n()}</Show>
    </span>
  );
  return (
    <div class="rtabs">
      {tab("files", "Ficheiros")}
      {tab("changes", "Alterações")}
      {tab("pulls", "PRs")}
    </div>
  );
}
