import { Show } from "solid-js";
import { Button } from "./ui";
import { tabKeys } from "./a11y";
import { K } from "./keyboard";
import { PanelIcon } from "./PanelIcon";
import { changes, rightTab, setRightTab, toggleRightPanel } from "./store";

/** Tab strip shared by every right-hand panel. */
export function RightTabs() {
  const n = () => changes()?.files.length ?? 0;
  const tab = (id: "files" | "changes" | "pulls", label: string) => (
    <button
      type="button"
      class="rtab"
      classList={{ active: rightTab() === id }}
      role="tab"
      id={`rtab-${id}`}
      aria-selected={rightTab() === id}
      aria-controls="rpanel"
      tabindex={rightTab() === id ? 0 : -1}
      onClick={() => setRightTab(id)}
    >
      {label}
      <Show when={id === "changes" && n()}> {n()}</Show>
    </button>
  );
  return (
    <div class="rtabs">
      <div class="rtablist" role="tablist" aria-label="Painel direito" onKeyDown={(e) => tabKeys(e, true)}>
        {tab("files", "Ficheiros")}
        {tab("changes", "Alterações")}
        {tab("pulls", "PRs")}
      </div>
      <span class="grow" />
      <Button variant="ghost" size="icon" class="icon add on" title={K("Esconder painel direito (Ctrl+Shift+B)")} aria-label={K("Esconder painel direito (Ctrl+Shift+B)")} onClick={toggleRightPanel}>
        <PanelIcon side="right" on={true} />
      </Button>
    </div>
  );
}
