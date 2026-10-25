import { For, Show, createResource, createSignal, onMount } from "solid-js";
import { clipboardWrite, diagnostics, hostInfo } from "./api";
import { restartDaemonFlow } from "./daemonActions";
import { aboutInfo, aboutRows, aboutText, chipText, fmtBuilt, restartHelps, skewWarning } from "./aboutLogic";
import { setError, setSettingsFocus, settingsFocus, showNotice, openAbout } from "./store";
import { errorText } from "./appError";
import { S } from "./strings";
import { L } from "./strings.logic";
import { Button } from "./ui";

/** What the host and the daemon report about their versions. A part that cannot be read shows as unknown. */
const load = async () => {
  const d = await diagnostics();
  const h = await hostInfo("local").catch((e) => (console.debug("host.info", e), null));
  return aboutInfo(d, h);
};

/** Settings > About: the app and daemon versions, a warning when they differ, and a copy button for bug reports. */
export function AboutSection() {
  const [info, { refetch }] = createResource(load);
  const [restarting, setRestarting] = createSignal(false);
  let sec!: HTMLElement;
  onMount(() => {
    if (settingsFocus() !== "about") return;
    setSettingsFocus(null);
    sec.scrollIntoView({ block: "start" });
  });
  const rows = (title: string, list: [string, string][]) => (
    <>
      <dt class="set-diag-head">{title}</dt>
      <dd />
      <For each={list}>
        {([k, v]) => (
          <>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </>
        )}
      </For>
    </>
  );
  return (
    <section class="set-sec" ref={sec} data-testid="about">
      <h4>{L.about.title}</h4>
      <p class="auto-note">{L.about.note}</p>
      <Show when={!info.error && info()} fallback={<div class="auto-note">{S.loading}</div>}>
        {(a) => (
          <>
            <Show
              when={skewWarning(a())}
              fallback={
                <div class="auto-note" data-testid="about-sync">
                  {L.about.inSync}
                </div>
              }
            >
              {(w) => (
                <div class="auto-note about-warn" role="status" data-testid="about-warn">
                  {w()}
                </div>
              )}
            </Show>
            <dl class="set-diag">
              {rows(L.about.app, aboutRows(a()).app)}
              {rows(L.about.daemon, aboutRows(a()).daemon)}
            </dl>
            <div class="m-actions start">
              <Show when={restartHelps(a())}>
                <Button
                  variant="default"
                  disabled={restarting()}
                  onClick={() => void restartDaemonFlow(setRestarting, () => void refetch())}
                >
                  {restarting() ? S.restartingDaemon : S.restartDaemon}
                </Button>
              </Show>
              <Button
                onClick={() =>
                  void clipboardWrite(aboutText(a())).then(
                    () => showNotice(L.about.copied),
                    (e) => setError(errorText(e)),
                  )
                }
              >
                {L.about.copy}
              </Button>
            </div>
          </>
        )}
      </Show>
    </section>
  );
}

/** Discreet "v0.1.0 · a1b2c3d" at the end of the status bar; a click opens Settings > About. */
export function VersionChip() {
  const [info] = createResource(() => diagnostics().catch((e) => (console.debug("diagnostics", e), null)));
  return (
    <Show when={info()}>
      {(d) => {
        const a = () => ({
          appVersion: d().app_version,
          appCommit: d().app_commit && d().app_commit !== "unknown" ? d().app_commit! : null,
        });
        return (
          <Button
            variant="row"
            size="xs"
            class="sb-item cursor-pointer hover:bg-foreground/10 hover:text-foreground"
            title={L.about.chipTip(
              `v${d().app_version}`,
              a().appCommit ?? L.about.unknown,
              fmtBuilt(d().app_built ?? null),
            )}
            aria-label={L.about.chipLabel}
            data-testid="version-chip"
            onClick={openAbout}
          >
            {chipText(a())}
          </Button>
        );
      }}
    </Show>
  );
}
