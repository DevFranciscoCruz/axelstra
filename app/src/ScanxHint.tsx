import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { scanxHintDismiss, scanxStatus } from "./api";
import { Button } from "./ui";
import { Icon } from "./Icon";
import { createLive } from "./live";
import { hintVisible, setPerfFocus, perfFocus } from "./scanxLogic";
import { setShowSettings } from "./store";
import { PF } from "./strings.perf";

/**
 * The one-time notice about the Defender exclusion. It only offers to open Settings > Performance;
 * it never applies anything. The daemon decides when it is pending (`status.hint`).
 */
export function ScanxHint() {
  const [show, setShow] = createSignal(false);
  const load = () =>
    scanxStatus().then(
      (s) => setShow(hintVisible(s)),
      () => undefined,
    );
  const live = createLive(load, ["scanx.changed"], { onFocus: false });
  onMount(() => live.now());
  onCleanup(() => live.stop());

  const open = () => {
    setShowSettings(true);
    setPerfFocus(perfFocus() + 1);
  };
  const dismiss = () => {
    setShow(false);
    void scanxHintDismiss().catch(() => undefined);
  };
  return (
    <Show when={show()}>
      <div class="update-toast flow" role="status" aria-label={PF.hintLabel}>
        <div class="ut-head">
          <Icon name="download" size={16} />
          <div>
            <p class="ut-title">{PF.hintTitle}</p>
            <p class="ut-sub">{PF.hintSub}</p>
          </div>
        </div>
        <div class="ut-actions">
          <Button onClick={dismiss}>{PF.hintDismiss}</Button>
          <Button variant="default" onClick={open}>
            {PF.hintOpen}
          </Button>
        </div>
      </div>
    </Show>
  );
}
