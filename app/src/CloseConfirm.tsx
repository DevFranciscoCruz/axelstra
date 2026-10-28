import { S } from "./strings";
import { U } from "./strings.ui";
import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Modal } from "./Modal";
import { sessions } from "./store";
import { Button, CheckboxField } from "./ui";
import { createPersistedFlag } from "./persist";

const [wanted, setWanted] = createPersistedFlag("axelstra.confirmClose", true);
const [keepWanted, setKeepWanted] = createPersistedFlag("axelstra.keepBackground", false);
const quit = (keep = keepWanted()) => void invoke("app_quit", { keep });

/** the host cancels every window close and asks here first; the terminals live in the daemon and survive */
export function CloseConfirm() {
  const [open, setOpen] = createSignal(false);
  const [skip, setSkip] = createSignal(false);
  const [keep, setKeep] = createSignal(keepWanted());
  onMount(() => {
    let un: (() => void) | undefined;
    void listen("close-requested", () => {
      if (wanted()) setOpen(true);
      else quit();
    }).then((u) => (un = u));
    onCleanup(() => un?.());
  });
  const running = () => sessions().filter((s) => s.running).length;
  const confirm = () => {
    if (skip()) setWanted(false);
    setKeepWanted(keep());
    quit(keep());
  };
  return (
    <Show when={open()}>
      <Modal label={U.closeConfirm.label} role="alertdialog" describedBy="close-confirm-msg" onClose={() => setOpen(false)}>
        <div class="m-title font-semibold">{U.closeConfirm.title}</div>
        <div id="close-confirm-msg" class="auto-note">
          {running() === 0
            ? U.closeConfirm.none
            : keep()
              ? U.closeConfirm.keepOn(running())
              : U.closeConfirm.keepOff(running())}
        </div>
        <CheckboxField checked={keep()} onChange={setKeep}>{U.closeConfirm.keepBackground}</CheckboxField>
        <CheckboxField checked={skip()} onChange={setSkip}>{U.closeConfirm.dontAsk}</CheckboxField>
        <div class="m-actions">
          <Button onClick={() => setOpen(false)}>{S.cancel}</Button>
          <Button variant="default" ref={(el: HTMLButtonElement) => queueMicrotask(() => el.focus())} onClick={confirm}>
            {S.close}
          </Button>
        </div>
      </Modal>
    </Show>
  );
}
