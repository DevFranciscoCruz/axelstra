import { Show, createResource, createSignal } from "solid-js";
import { hooksInstall, hooksStatus } from "./api";
import { errorText } from "./settingsLogic";
import { showNotice } from "./store";
import { Button } from "./ui";

/** Settings block: whether Claude's hooks that report "waiting for an answer" are installed, and a button to install them. */
export function HooksSection() {
  const [st, { refetch }] = createResource(() => hooksStatus());
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const install = async () => {
    setBusy(true);
    setError("");
    try {
      const r = await hooksInstall();
      showNotice(r.changed ? `Hooks instalados${r.backup ? ` (cópia em ${r.backup})` : ""}` : "Os hooks já estavam instalados");
      await refetch();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section class="set-sec">
      <h4>Agentes: estado em tempo real</h4>
      <p class="auto-note">O Claude pode avisar o Axelstra quando fica à espera de uma resposta tua ou quando termina.</p>
      <p class="auto-note">Os hooks ficam em ~/.claude/settings.json; é feita uma cópia de segurança antes de alterar.</p>
      <div class="set-hooks" aria-live="polite">
        <Show when={!st.error && st()} fallback={<span class="auto-note">{st.error ? "Estado dos hooks indisponível (daemon desatualizado?)." : "A carregar…"}</span>}>
          {(v) => <span>{v().installed ? "Hooks instalados" : "Hooks não instalados"}</span>}
        </Show>
        <Show when={!st()?.installed}>
          <Button disabled={busy()} onClick={() => void install()}>
            {busy() ? "A instalar…" : "Instalar hooks do Claude"}
          </Button>
        </Show>
      </div>
      <Show when={error()}>
        <div class="set-err" role="alert">
          Não foi possível instalar os hooks: {error()}
        </div>
      </Show>
    </section>
  );
}
