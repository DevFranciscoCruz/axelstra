import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { Show, createSignal, onCleanup, onMount } from "solid-js";

type Info = { version: string; notes: string | null };

const HOUR = 60 * 60 * 1000;

export function UpdateBanner() {
  const [info, setInfo] = createSignal<Info | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [err, setErr] = createSignal("");

  const check = async () => {
    try {
      setInfo(await invoke<Info | null>("update_check"));
    } catch {
      /* offline or no release published yet: stay quiet, retry later */
    }
  };
  onMount(() => {
    const first = setTimeout(() => void check(), 5000);
    const again = setInterval(() => void check(), 6 * HOUR);
    onCleanup(() => {
      clearTimeout(first);
      clearInterval(again);
    });
  });

  const install = async () => {
    const i = info();
    if (!i) return;
    const ok = await ask(
      `Instalar a versão ${i.version}?\n\nA app reinicia e os terminais e agentes em execução são fechados (o trabalho nos ficheiros fica).`,
      { title: "Atualizar Axelstra", kind: "warning" },
    );
    if (!ok) return;
    setBusy(true);
    setErr("");
    try {
      await invoke("update_install", { expected: i.version });
    } catch (e) {
      setErr(String(e));
      setBusy(false);
    }
  };

  return (
    <Show when={info()}>
      {(i) => (
        <div class="update">
          <button class="primary" disabled={busy()} title={i().notes ?? ""} onClick={() => void install()}>
            {busy() ? "A atualizar…" : `Atualizar para ${i().version}`}
          </button>
          <Show when={err()}>
            <div class="error">{err()}</div>
          </Show>
        </div>
      )}
    </Show>
  );
}
