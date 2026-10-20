import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Modal } from "./Modal";
import { sessions } from "./store";

const KEY = "axelstra.confirmClose";
const wanted = () => {
  try {
    return localStorage.getItem(KEY) !== "0";
  } catch {
    return true;
  }
};

const quit = () => void invoke("app_quit");

/** the host cancels every window close and asks here first; the terminals live in the daemon and survive */
export function CloseConfirm() {
  const [open, setOpen] = createSignal(false);
  const [skip, setSkip] = createSignal(false);
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
    try {
      if (skip()) localStorage.setItem(KEY, "0");
    } catch {
      /* preference only */
    }
    quit();
  };
  return (
    <Show when={open()}>
      <Modal label="Fechar janela" onClose={() => setOpen(false)}>
        <div class="m-title" style={{ "font-weight": "600" }}>Fechar o Axelstra?</div>
        <div class="auto-note">
          {running() > 0
            ? `${running()} ${running() === 1 ? "sessão em execução continua" : "sessões em execução continuam"} em segundo plano e ${running() === 1 ? "volta" : "voltam"} quando abrir a aplicação outra vez.`
            : "Não há sessões em execução."}
        </div>
        <label class="voice-auto" style={{ "align-self": "flex-start" }}>
          <input type="checkbox" checked={skip()} onChange={(e) => setSkip(e.currentTarget.checked)} /> Não perguntar novamente
        </label>
        <div class="m-actions" style={{ "justify-content": "flex-end" }}>
          <button onClick={() => setOpen(false)}>Cancelar</button>
          <button class="primary" ref={(el) => queueMicrotask(() => el.focus())} onClick={confirm}>
            Fechar
          </button>
        </div>
      </Modal>
    </Show>
  );
}
