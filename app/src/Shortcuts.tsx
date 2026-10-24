import { For } from "solid-js";
import { cheatSheet } from "./commands";
import { Modal } from "./Modal";
import { K } from "./keyboard";
import { setShowShortcuts } from "./store";
import { Button } from "./ui";

/** Cheat sheet with every keyboard shortcut (opened with ?), grouped; fed by the command registry like the palette */
export function Shortcuts() {
  const close = () => setShowShortcuts(false);
  return (
    <Modal label="Atalhos de teclado" class="modal settings" onClose={close}>
      <div class="auto-head">
        <strong>Atalhos de teclado</strong>
        <span class="grow" />
        <Button onClick={close}>Fechar</Button>
      </div>
      <div class="disp-list">
        <For each={cheatSheet()}>
          {(g) => (
            <>
              <div class="auto-note" role="heading" aria-level="3">
                {g.title}
              </div>
              <For each={g.rows}>
                {([keys, what]) => (
                  <div class="disp-head">
                    <kbd>{K(keys)}</kbd>
                    <span class="auto-note">{what}</span>
                  </div>
                )}
              </For>
            </>
          )}
        </For>
      </div>
    </Modal>
  );
}
