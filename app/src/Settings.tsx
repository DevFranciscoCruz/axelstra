import { For, JSX, Show, createResource, createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { K } from "./keyboard";
import {
  NotifyMode,
  SettingsPatch,
  budgetCheck,
  ShellChoice,
  Terse,
  backupExport,
  worktreeSweepFull,
  SweepItem,
  pickAndRestoreBackup,
  copyDiagnostics,
  diagnostics,
  getSettings,
  resetShellCache,
  setSettings,
} from "./api";
import { restartDaemonFlow } from "./daemonActions";
import { BudgetsSection } from "./BudgetsSection";
import { setBudgetRows } from "./budgetStore";
import { AboutSection } from "./About";
import { HooksSection } from "./HooksSection";
import { PerformanceSection } from "./PerformanceSection";
import { WorkspaceSection } from "./WorkspaceSection";
import { HostsSection } from "./HostsSection";
import { Modal } from "./Modal";
import { S } from "./strings";
import { classifySettingsError, diagnosticRows, errorText } from "./settingsLogic";
import { refreshProjects, setError, setShowSettings, showNotice } from "./store";
import { DEFAULT_FONT, MAX_FONT, MIN_FONT, TermTheme, setTermPrefs, termPrefs } from "./termprefs";
import { APP_THEMES, appTheme, setAppTheme } from "./theme";
import { LOCALES, LOCALE_NAMES, locale, setLocale } from "./i18n";
import { L } from "./strings.logic";
import { P } from "./strings.panels";
import { ask } from "./Confirm";
import "./termui.css";
import { Button, CheckboxField, Field, Input, RadioGroup } from "./ui";

const TERSE: Terse[] = ["off", "short", "tiny"];
const STALL_MINUTES = [0, 2, 5, 10, 15, 30];
const NOTIFY: NotifyMode[] = ["all", "waiting", "off"];
const SHELLS: ShellChoice[] = ["auto", "pwsh", "powershell", "bash", "wsl", "cmd"];
const THEMES: TermTheme[] = ["dark", "light"];

// texts of the backup / diagnostics / accessibility rows
const T = L.settingsRows;

/** A row of exclusive choices (a null value, "auto", is keyed by a sentinel: radio values are strings). */
const NONE = "__none__";
function Choices<T extends string | null>(props: {
  label: string;
  value: T;
  options: { value: T; label: string; hint?: string }[];
  onPick: (v: T) => void;
}) {
  const key = (v: T) => v ?? NONE;
  return (
    <RadioGroup
      label={props.label}
      value={key(props.value)}
      options={props.options.map((o) => ({ value: key(o.value), label: o.label, hint: o.hint }))}
      onChange={(k) => props.onPick(props.options.find((o) => key(o.value) === k)!.value)}
    />
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
    const info = await diagnostics().catch((e) => (console.debug("diagnostics", e), null));
    const missing =
      info?.missing_tools ??
      (await invoke<string[]>("missing_tools").catch((e) => (console.debug("missing_tools", e), null)));
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
      <div class="m-actions start">
        <Button onClick={() => void refetch()}>{S.diagRun}</Button>
        <Button
          onClick={() =>
            void copyDiagnostics().then(
              () => showNotice(T.diagCopied),
              (e) => setError(errorText(e)),
            )
          }
        >
          {T.copyDiag}
        </Button>
      </div>
    </Section>
  );
}

/** App-wide preferences. Each change is saved at once; agents already open keep their current behaviour. */
export function Settings() {
  const [s, { mutate, refetch }] = createResource(() => getSettings());
  const [restarting, setRestarting] = createSignal(false);
  const save = async (patch: SettingsPatch) => {
    try {
      await setSettings(patch);
      mutate((cur) => (cur ? { ...cur, ...patch } : cur));
      if (patch.shell !== undefined) resetShellCache();
    } catch (e) {
      setError(errorText(e));
    }
  };
  const exportBackup = async () => {
    try {
      const r = await backupExport();
      if (r) showNotice(T.exported + r.path);
      else showNotice(T.noBackup);
    } catch (e) {
      setError(errorText(e));
    }
  };
  const restoreBackup = async () => {
    // restoring replaces settings, projects and sessions: ask before even opening the folder dialog
    if (!(await ask(P.flows.restoreAsk, { title: P.flows.restoreTitle, kind: "warning", okLabel: P.flows.restoreOk })))
      return;
    try {
      const r = await pickAndRestoreBackup(T.restorePath);
      if (r === undefined) return; // dialog cancelled
      showNotice(r ? T.restored : T.noBackup);
    } catch (e) {
      setError(errorText(e));
    }
  };
  const [swept, setSwept] = createSignal<SweepItem[] | null>(null);
  const [closedNote, setClosedNote] = createSignal("");
  const cleanLine = (i: SweepItem) =>
    [i.warning ? T.cleanWarning : i.removed ? T.cleanRemoved : "", i.branch || i.path, i.warning ?? i.why ?? ""]
      .filter(Boolean)
      .join(" · ");
  const cleanNow = async () => {
    // same operation as "Limpar" in the sidebar, which asks first: it closes sessions and removes worktrees of every project
    if (
      !(await ask(P.flows.cleanNowAsk, { title: P.flows.cleanNowTitle, kind: "danger", okLabel: P.flows.cleanNowOk }))
    )
      return;
    try {
      const r = await worktreeSweepFull();
      setSwept(r.swept);
      setClosedNote(
        r.closed_sessions || r.closed_procs
          ? T.cleanClosed(r.closed_sessions ?? 0, r.closed_procs ?? 0, r.freed_bytes ?? 0)
          : "",
      );
      await refreshProjects();
    } catch (e) {
      setError(errorText(e));
    }
  };
  const restartDaemon = () => void restartDaemonFlow(setRestarting, () => void refetch());
  return (
    <Modal label={S.settings} class="modal settings" onClose={() => setShowSettings(false)}>
      <div class="auto-head">
        <strong>{S.settings}</strong>
        <span class="grow" />
        <Button onClick={() => setShowSettings(false)}>{S.close}</Button>
      </div>
      <Show
        when={s.state !== "errored"}
        fallback={
          <div class="set-error">
            <div class="auto-note">
              {classifySettingsError(s.error) === "old-daemon" ? S.oldDaemon : S.loadFailed + errorText(s.error)}
            </div>
            <div class="m-actions">
              <Show when={classifySettingsError(s.error) === "old-daemon"}>
                <Button variant="default" disabled={restarting()} onClick={restartDaemon}>
                  {restarting() ? S.restartingDaemon : S.restartDaemon}
                </Button>
              </Show>
              <Button disabled={restarting()} onClick={() => void refetch()}>
                {S.retry}
              </Button>
            </div>
          </div>
        }
      >
        <Show when={s()} fallback={<div class="auto-note">{S.loading}</div>}>
          {(cfg) => (
            <>
              <AboutSection />

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
                    options={[
                      { value: null, label: S.auto },
                      ...cfg().installed.map((m) => ({ value: m as string | null, label: m })),
                    ]}
                    onPick={(v) => void save({ pm: v })}
                  />
                </Show>
                <CheckboxField checked={cfg().auto_install} onChange={(v) => void save({ auto_install: v })}>
                  {S.autoInstall}
                </CheckboxField>
              </Section>

              <Section title={S.notify} note={S.notifyNote}>
                <Choices
                  label={S.notify}
                  value={cfg().notify_mode ?? "all"}
                  options={NOTIFY.map((v) => ({ value: v, label: S.notifyMode[v].label, hint: S.notifyMode[v].hint }))}
                  onPick={(v) => void save({ notify_mode: v })}
                />
                <CheckboxField checked={cfg().notify_sound ?? true} onChange={(v) => void save({ notify_sound: v })}>
                  {S.notifySound}
                </CheckboxField>
              </Section>

              <Section title={S.stall} note={S.stallNote}>
                <Choices
                  label={S.stall}
                  value={String(cfg().stall_minutes ?? 5)}
                  options={[...new Set([...STALL_MINUTES, cfg().stall_minutes ?? 5])]
                    .sort((a, b) => a - b)
                    .map((v) => ({ value: String(v), label: v === 0 ? S.stallOff : S.stallMin(v) }))}
                  onPick={(v) => void save({ stall_minutes: Number(v) })}
                />
              </Section>

              <Section title={S.shell} note={S.shellNote}>
                <Choices
                  label={S.shell}
                  value={cfg().shell ?? "auto"}
                  options={SHELLS.map((v) => ({ value: v, label: S.shells[v] }))}
                  onPick={(v) => void save({ shell: v })}
                />
              </Section>

              <Section title={L.lang.title} note={L.lang.note}>
                <Choices
                  label={L.lang.title}
                  value={locale()}
                  options={LOCALES.map((v) => ({ value: v, label: LOCALE_NAMES[v] }))}
                  onPick={setLocale}
                />
              </Section>

              <Section title={S.appearance} note={S.appearanceNote}>
                <Choices
                  label={S.appTheme}
                  value={appTheme()}
                  options={APP_THEMES.map((v) => ({ value: v, label: S.appThemes[v] }))}
                  onPick={setAppTheme}
                />
              </Section>

              <Section title={S.terminal} note={K(S.terminalNote)}>
                <Choices
                  label={S.termTheme}
                  value={termPrefs().theme}
                  options={THEMES.map((v) => ({ value: v, label: S.termThemes[v] }))}
                  onPick={(v) => setTermPrefs({ theme: v })}
                />
                <div class="set-fields">
                  <Field label={S.termFont}>
                    <Input
                      type="text"
                      value={termPrefs().fontFamily}
                      placeholder={DEFAULT_FONT}
                      spellcheck={false}
                      onChange={(e) => setTermPrefs({ fontFamily: e.currentTarget.value })}
                    />
                  </Field>
                  <Field label={S.termFontSize}>
                    <Input
                      type="number"
                      min={MIN_FONT}
                      max={MAX_FONT}
                      value={termPrefs().fontSize}
                      onChange={(e) => setTermPrefs({ fontSize: Number(e.currentTarget.value) })}
                    />
                  </Field>
                </div>
              </Section>

              <Section title={T.a11y} note={T.a11yNote}>
                <CheckboxField checked={termPrefs().screenReader} onChange={(v) => setTermPrefs({ screenReader: v })}>
                  {T.screenReader}
                </CheckboxField>
              </Section>

              <Section title={T.cleanTitle} note={T.cleanNote}>
                <div class="m-actions start">
                  <Button onClick={() => void cleanNow()}>{T.cleanNow}</Button>
                </div>
                <Show when={swept()}>
                  {(list) => (
                    <ul class="clean-list">
                      <Show when={closedNote()}>
                        <li>{closedNote()}</li>
                      </Show>
                      <Show when={!list().length && !closedNote()}>
                        <li>{T.cleanNothing}</li>
                      </Show>
                      <For each={list()}>{(i) => <li classList={{ warn: !!i.warning }}>{cleanLine(i)}</li>}</For>
                    </ul>
                  )}
                </Show>
              </Section>

              <Section title={T.backup} note={T.backupNote}>
                <div class="m-actions start">
                  <Button onClick={() => void exportBackup()}>{T.export}</Button>
                  <Button onClick={() => void restoreBackup()}>{T.restore}</Button>
                </div>
              </Section>

              <BudgetsSection
                budgets={cfg().budgets}
                onSave={(b) => void save({ budgets: b }).then(() => budgetCheck().then(setBudgetRows, () => undefined))}
              />
              <HostsSection />
              <WorkspaceSection />

              <HooksSection />
              <PerformanceSection />

              <Diagnostics />
            </>
          )}
        </Show>
      </Show>
    </Modal>
  );
}
