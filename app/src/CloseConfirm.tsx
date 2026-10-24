import { count, plural } from "./strings";
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
      <Modal label="Fechar janela" onClose={() => setOpen(false)}>
        <div class="m-title" style={{ "font-weight": "600" }}>Fechar o Axelstra?</div>
        <div class="auto-note">
          {running() === 0
            ? "Não há sessões em execução."
            : keep()
              ? `${count(running(), "sessão continua", "sessões continuam")} a correr em segundo plano (e consomem recursos) até voltar a abrir a aplicação.`
              : `${count(running(), "sessão é fechada", "sessões são fechadas")} e nada fica a consumir. ${plural(running(), "Volta", "Voltam")} quando abrir a aplicação outra vez; os agentes retomam a conversa.`}
        </div>
        <CheckboxField checked={keep()} onChange={setKeep}>Manter sessões e automatizações a correr em segundo plano</CheckboxField>
        <CheckboxField checked={skip()} onChange={setSkip}>Não perguntar novamente</CheckboxField>
        <div class="m-actions" style={{ "justify-content": "flex-end" }}>
          <Button onClick={() => setOpen(false)}>Cancelar</Button>
          <Button variant="default" ref={(el: HTMLButtonElement) => queueMicrotask(() => el.focus())} onClick={confirm}>
            Fechar
          </Button>
        </div>
      </Modal>
    </Show>
  );
}
