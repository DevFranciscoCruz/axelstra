import { Show } from "solid-js";
import { Button, Tabs, TabsList, TabsTrigger } from "./ui";
import { K } from "./keyboard";
import { PanelIcon } from "./PanelIcon";
import { changes, rightTab, setRightTab, toggleRightPanel } from "./store";

type RightTab = "files" | "changes" | "pulls";

/** Tab strip shared by every right-hand panel. */
export function RightTabs() {
  const n = () => changes()?.files.length ?? 0;
  const tab = (id: RightTab, label: string) => (
    <TabsTrigger value={id} id={`rtab-${id}`} aria-controls="rpanel">
      {label}
      <Show when={id === "changes" && n()}> {n()}</Show>
    </TabsTrigger>
  );
  return (
    <Tabs value={rightTab()} onChange={(v) => setRightTab(v as RightTab)} class="flex gap-0.5 border-b border-[var(--line)] px-2 pt-1.5">
      <TabsList aria-label="Painel direito">
        {tab("files", "Ficheiros")}
        {tab("changes", "Alterações")}
        {tab("pulls", "PRs")}
      </TabsList>
      <span class="flex-1" />
      <Button variant="icon" size="icon" class="text-foreground" title={K("Esconder painel direito (Ctrl+Shift+B)")} aria-label={K("Esconder painel direito (Ctrl+Shift+B)")} onClick={toggleRightPanel}>
        <PanelIcon side="right" on={true} />
      </Button>
    </Tabs>
  );
}
