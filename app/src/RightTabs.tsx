import { Show } from "solid-js";
import { Button, Tabs, TabsList, TabsTrigger } from "./ui";
import { K } from "./keyboard";
import { P } from "./strings.panels";
import { U } from "./strings.ui";
import { PanelIcon } from "./PanelIcon";
import { changes, rightTab, setRightTab, toggleRightPanel } from "./store";

type RightTab = "files" | "changes" | "pulls" | "timeline";

/** Tab strip shared by every right-hand panel. */
export function RightTabs() {
  const n = () => changes()?.files.length ?? 0;
  const tab = (id: RightTab, label: string) => (
    <TabsTrigger
      value={id}
      id={`rtab-${id}`}
      aria-controls="rpanel"
      title={label}
      class="shrink-0 whitespace-nowrap"
    >
      {label}
      <Show when={id === "changes" && n()}> {n()}</Show>
    </TabsTrigger>
  );
  return (
    <Tabs
      value={rightTab()}
      onChange={(v) => setRightTab(v as RightTab)}
      class="flex gap-0.5 border-b border-[var(--line)] px-2 pt-1.5"
    >
      <TabsList aria-label={U.rightTabs.label} class="min-w-0 flex-[0_1_auto] overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {tab("files", U.rightTabs.files)}
        {tab("changes", U.rightTabs.changes)}
        {tab("pulls", U.rightTabs.prs)}
        {tab("timeline", P.timeline.tab)}
      </TabsList>
      <span class="flex-1 shrink-0" />
      <Button
        variant="icon"
        size="icon"
        class="shrink-0 text-foreground"
        title={K(U.rightTabs.hide)}
        aria-label={K(U.rightTabs.hide)}
        onClick={toggleRightPanel}
      >
        <PanelIcon side="right" on={true} />
      </Button>
    </Tabs>
  );
}
