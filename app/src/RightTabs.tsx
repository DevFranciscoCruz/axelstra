import { Show } from "solid-js";
import { press } from "./a11y";
import { PanelIcon } from "./PanelIcon";
import { changes, rightTab, setRightTab, toggleRightPanel } from "./store";

/** Tab strip shared by every right-hand panel. */
export function RightTabs() {
  const n = () => changes()?.files.length ?? 0;
  const tab = (id: "files" | "changes" | "pulls", label: string) => (
    <span class="rtab" classList={{ active: rightTab() === id }} aria-selected={rightTab() === id} {...press(() => setRightTab(id), "tab")}>
      {label}
      <Show when={id === "changes" && n()}> {n()}</Show>
    </span>
  );
  return (
    <div class="rtabs" role="tablist">
      {tab("files", "Ficheiros")}
      {tab("changes", "Alterações")}
      {tab("pulls", "PRs")}
      <span class="grow" />
      <button class="icon add on" title="Esconder painel direito (Ctrl+Shift+B)" aria-label="Esconder painel direito (Ctrl+Shift+B)" onClick={toggleRightPanel}>
        <PanelIcon side="right" on={true} />
      </button>
    </div>
  );
}
