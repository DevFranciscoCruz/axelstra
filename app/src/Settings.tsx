import { For, JSX, Show, createResource, createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { NotifyMode, SettingsPatch, ShellChoice, Terse, call, daemonShutdown, diagnostics, getSettings, resetShellCache, setSettings } from "./api";
import { radioKeys } from "./a11y";
import { ask } from "./Confirm";
import { Modal } from "./Modal";
import { S } from "./strings";
import { classifySettingsError, diagnosticRows, errorText, waitUntil } from "./settingsLogic";
import { setShowSettings, showNotice } from "./store";
import { DEFAULT_FONT, MAX_FONT, MIN_FONT, TermTheme, setTermPrefs, termPrefs } from "./termprefs";
import "./termui.css";

const TERSE: Terse[] = ["off", "short", "tiny"];
const NOTIFY: NotifyMode[] = ["all", "waiting", "off"];
const SHELLS: ShellChoice[] = ["auto", "pwsh", "powershell", "bash", "wsl", "cmd"];
const THEMES: TermTheme[] = ["dark", "light"];

/** A row of exclusive choices: one tab stop (the checked one), arrow keys move and select. */
function Choices<T extends string | null>(props: { label: string; value: T; options: { value: T; label: string; hint?: string }[]; onPick: (v: T) => void }) {
  return (
    <div class="set-choices" role="radiogroup" aria-label={props.label} onKeyDown={radioKeys}>
      <For each={props.options}>
        {(o) => (
          <button
            role="radio"
            aria-checked={props.value === o.value}
            tabindex={props.value === o.value ? 0 : -1}
            classList={{ on: props.value === o.value }}
            title={o.hint}
            onClick={() => props.onPick(o.value)}
          >
            {o.label}
          </button>
        )}
      </For>
    </div>
  );
}

const Section = (props: { title: string; note?: string; children: JSX.Element }) => (
  <section class="set-sec">
    <h4>{props.title}</h4>
    <Show when={props.note}>
      <p class="auto-note">{props.note}</p>
    </Show>
    {props.children}
  </section>
);

/** Daemon version, data paths, lag log and missing tools. Every part is optional: an older build reports less. */
function Diagnostics() {
  const [d, { refetch }] = createResource(async () => {
    const info = await diagnostics().catch(() => null);
    const missing = info?.missing_tools ?? (await invoke<string[]>("missing_tools").catch(() => null));
    return { info, missing };
  });
  return (
    <Section title={S.diag} note={S.diagNote}>
      <Show when={!d.error && d()} fallback={<div class="auto-note">{S.loading}</div>}>
        {(r) => (
          <>
            <Show when={!r().info}>
              <div class="auto-note">{S.diagUnavailable}</div>
            </Show>
            <dl class="set-diag">
              <For each={diagnosticRows(r().info)}>
                {(row) => (
                  <>
                    <dt>{row.label}</dt>
                    <dd>{row.value}</dd>
                  </>
                )}
              </For>
              <Show when={r().missing}>
                <dt>{S.diagMissing}</dt>
                <dd>{r().missing!.length ? r().missing!.join(", ") : S.diagNoneMissing}</dd>
              </Show>
            </dl>
          </>
        )}
      </Show>
      <div>
        <button onClick={() => void refetch()}>{S.diagRun}</button>
      </div>
    </Section>
  );
}

/** App-wide preferences. Each change is saved at once; agents already open keep their current behaviour. */
export function Settings() {
  const [s, { mutate, refetch }] = createResource(getSettings);
  const [restarting, setRestarting] = createSignal(false);
  const save = async (patch: SettingsPatch) => {
    try {
      await setSettings(patch);
      mutate((cur) => (cur ? { ...cur, ...patch } : cur));
      if (patch.shell !== undefined) resetShellCache();
    } catch (e) {
      showNotice(String(e));
    }
  };
  // The daemon is a separate process that outlives the app. Shutting it down is safe cross-platform: the next call starts
  // a fresh one (and the app restores the sessions, see "daemon-restarted").
  const restartDaemon = async () => {
    if (!(await ask(S.restartConfirm, { title: S.restartTitle, kind: "warning", okLabel: S.restartDaemon }))) return;
    setRestarting(true);
    try {
      await daemonShutdown().catch(() => 0); // the connection drops with the daemon
      if (!(await waitUntil(() => call("ping"), 20, 500))) throw new Error("o daemon não voltou a responder");
      await refetch();
    } catch (e) {
      showNotice(S.restartFailed + errorText(e));
    } finally {
      setRestarting(false);
    }
  };
  return (
    <Modal label={S.settings} class="modal settings" onClose={() => setShowSettings(false)}>
      <div class="auto-head">
        <strong>{S.settings}</strong>
        <span class="grow" />
        <button onClick={() => setShowSettings(false)}>{S.close}</button>
      </div>
      <Show
        when={s.state !== "errored"}
        fallback={
          <div class="set-error">
            <div class="auto-note">{classifySettingsError(s.error) === "old-daemon" ? S.oldDaemon : S.loadFailed + errorText(s.error)}</div>
            <div class="m-actions">
              <Show when={classifySettingsError(s.error) === "old-daemon"}>
                <button class="primary" disabled={restarting()} onClick={() => void restartDaemon()}>
                  {restarting() ? S.restartingDaemon : S.restartDaemon}
                </button>
              </Show>
              <button disabled={restarting()} onClick={() => void refetch()}>
                {S.retry}
              </button>
            </div>
          </div>
        }
      >
        <Show when={s()} fallback={<div class="auto-note">{S.loading}</div>}>
          {(cfg) => (
            <>
              <Section title={S.replies} note={S.repliesNote}>
                <Choices
                  label={S.replies}
                  value={cfg().terse}
                  options={TERSE.map((v) => ({ value: v, label: S.terse[v].label, hint: S.terse[v].hint }))}
                  onPick={(v) => void save({ terse: v })}
                />
                <div class="auto-note">{S.terse[cfg().terse].hint}</div>
              </Section>

              <Section title={S.pm} note={S.pmNote}>
                <Show when={cfg().installed.length > 0} fallback={<div class="auto-note">{S.pmNone}</div>}>
                  <Choices<string | null>
                    label={S.pm}
                    value={cfg().pm}
                    options={[{ value: null, label: S.auto }, ...cfg().installed.map((m) => ({ value: m as string | null, label: m }))]}
                    onPick={(v) => void save({ pm: v })}
                  />
                </Show>
                <label class="set-check">
                  <input type="checkbox" checked={cfg().auto_install} onChange={(e) => void save({ auto_install: e.currentTarget.checked })} />
                  {S.autoInstall}
                </label>
              </Section>

              <Section title={S.notify} note={S.notifyNote}>
                <Choices
                  label={S.notify}
                  value={cfg().notify_mode ?? "all"}
                  options={NOTIFY.map((v) => ({ value: v, label: S.notifyMode[v].label, hint: S.notifyMode[v].hint }))}
                  onPick={(v) => void save({ notify_mode: v })}
                />
                <label class="set-check">
                  <input type="checkbox" checked={cfg().notify_sound ?? true} onChange={(e) => void save({ notify_sound: e.currentTarget.checked })} />
                  {S.notifySound}
                </label>
              </Section>

              <Section title={S.shell} note={S.shellNote}>
                <Choices
                  label={S.shell}
                  value={cfg().shell ?? "auto"}
                  options={SHELLS.map((v) => ({ value: v, label: S.shells[v] }))}
                  onPick={(v) => void save({ shell: v })}
                />
              </Section>

              <Section title={S.terminal} note={S.terminalNote}>
                <Choices
                  label={S.termTheme}
                  value={termPrefs().theme}
                  options={THEMES.map((v) => ({ value: v, label: S.termThemes[v] }))}
                  onPick={(v) => setTermPrefs({ theme: v })}
                />
                <div class="set-fields">
                  <label>
                    {S.termFont}
                    <input
                      type="text"
                      value={termPrefs().fontFamily}
                      placeholder={DEFAULT_FONT}
                      spellcheck={false}
                      onChange={(e) => setTermPrefs({ fontFamily: e.currentTarget.value })}
                    />
                  </label>
                  <label>
                    {S.termFontSize}
                    <input
                      type="number"
                      min={MIN_FONT}
                      max={MAX_FONT}
                      value={termPrefs().fontSize}
                      onChange={(e) => setTermPrefs({ fontSize: Number(e.currentTarget.value) })}
                    />
                  </label>
                </div>
              </Section>

              <Diagnostics />
            </>
          )}
        </Show>
      </Show>
    </Modal>
  );
}
