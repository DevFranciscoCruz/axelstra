import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { Icon } from "../Icon";
import { pickProject } from "../projectActions";
import { AP } from "../strings.sources";
import { onTermDrop, dropInside } from "../terminal/dragDrop";
import { Button } from "../ui";
import { looksLikeFolder } from "./logic";

/** An existing folder: the native picker adds it (the page never names the path, see api.ts). A folder dropped on the zone only
 *  tells which one to pick, because the page may not hand a path to `project.add`. */
export function LocalTab(props: { onClose: () => void }) {
  const [busy, setBusy] = createSignal(false);
  const [dropped, setDropped] = createSignal("");
  let zone: HTMLDivElement | undefined;

  onMount(() => {
    try {
      const off = onTermDrop((p) => {
        if (!zone || !dropInside(zone, p.position)) return;
        const path = p.paths.find(looksLikeFolder) ?? p.paths[0];
        if (path)
          setDropped(
            path
              .replace(/[\\/]+$/, "")
              .split(/[\\/]/)
              .pop() || path,
          );
      });
      onCleanup(off);
    } catch (e) {
      // no webview events (a plain browser): the picker button still works
      console.debug("drag-drop", e);
    }
  });

  const pick = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      if (await pickProject()) props.onClose();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="flex flex-col gap-3">
      <div
        ref={zone}
        role="group"
        aria-label={AP.localDropZone}
        class="flex flex-col items-center gap-2 rounded-md border border-dashed border-border p-6 text-center"
      >
        <Icon name="folder" size={22} />
        <span class="text-base">{AP.localDrop}</span>
        <Button type="button" variant="default" data-ap-primary="" disabled={busy()} onClick={() => void pick()}>
          {busy() ? AP.busy : AP.localPick}
        </Button>
      </div>
      <div role="status" aria-live="polite" class="auto-note">
        <Show when={dropped()}>{(n) => AP.localDropFallback(n())}</Show>
      </div>
    </div>
  );
}
