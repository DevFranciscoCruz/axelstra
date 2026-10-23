import { For } from "solid-js";
import { Modal } from "./Modal";
import { K, SHORTCUTS } from "./keyboard";
import { setShowShortcuts } from "./store";

/** Cheat sheet with every keyboard shortcut (opened with ?) */
export function Shortcuts() {
  const close = () => setShowShortcuts(false);
  return (
    <Modal label="Atalhos de teclado" class="modal settings" onClose={close}>
      <div class="auto-head">
        <strong>Atalhos de teclado</strong>
        <span class="grow" />
        <button onClick={close}>Fechar</button>
      </div>
      <div class="disp-list">
        <For each={SHORTCUTS}>
          {([keys, what]) => (
            <div class="disp-head">
              <kbd>{K(keys)}</kbd>
              <span class="auto-note">{what}</span>
            </div>
          )}
        </For>
      </div>
    </Modal>
  );
}
