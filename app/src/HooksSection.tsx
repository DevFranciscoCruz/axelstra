import { For, Show, createResource, createSignal } from "solid-js";
import { agentsList, hookAgentsToShow, hookSettingsPath, hooksInstall, hooksStatus, type HookAgent } from "./api";
import { agentLabel } from "./agentLabel";
import { errorText } from "./settingsLogic";
import { showNotice } from "./store";
import { P } from "./strings.panels";
import { Button } from "./ui";

const H = P.hooks;

/** One agent's hooks: whether they are installed and a button to install them. */
function HooksBlock(props: { agent: HookAgent }) {
  const [st, { refetch }] = createResource(() => hooksStatus(props.agent));
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const name = () => agentLabel(props.agent);
  const install = async () => {
    setBusy(true);
    setError("");
    try {
      const r = await hooksInstall(props.agent);
      showNotice(r.changed ? H.done(name(), r.backup) : H.already(name()));
      await refetch();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div>
      <h5>{name()}</h5>
      <p class="auto-note">{H.pathNote(hookSettingsPath(props.agent))}</p>
      <div class="set-hooks" aria-live="polite">
        <Show when={!st.error && st()} fallback={<span class="auto-note">{st.error ? H.unavailable : H.loading}</span>}>
          {(v) => <span>{v().installed ? H.installed : H.notInstalled}</span>}
        </Show>
        <Show when={!st()?.installed}>
          <Button disabled={busy()} onClick={() => void install()}>
            {busy() ? H.installing : H.install(name())}
          </Button>
        </Show>
      </div>
      <Show when={error()}>
        <div class="set-err" role="alert">
          {H.failed(name(), error())}
        </div>
      </Show>
    </div>
  );
}

/** Settings block: for each agent, whether the hooks that report "waiting for an answer" are installed. */
export function HooksSection() {
  const [list] = createResource(() => agentsList().catch(() => undefined));
  return (
    <section class="set-sec">
      <h4>{H.title}</h4>
      <p class="auto-note">{H.intro}</p>
      <For each={hookAgentsToShow(list())}>{(a) => <HooksBlock agent={a} />}</For>
    </section>
  );
}
