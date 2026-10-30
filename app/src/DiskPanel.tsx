import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { diskStatus, sweepArtifacts, type DiskStatus, type DiskWorktree } from "./api";
import { errorText } from "./appError";
import { ask } from "./Confirm";
import { bySize, fmtMb, idleSecs, inactive, reclaimableTotal, staleList, wtName } from "./diskLogic";
import { createLive } from "./live";
import { coded } from "./msg";
import { integrateFlow } from "./sidebar/PendingIntegration";
import { pendingOf } from "./statusGroups";
import { overview, projects, selectWorktree } from "./store";
import { D } from "./strings.disk";
import { ago } from "./time";
import { toast } from "./toasts";
import { Button } from "./ui";

/** the same key as the janitor's own toast, so a manual clean and the event it raises show one toast */
const TOAST_KEY = "janitor:artifacts";

/** Sizes of the worktrees and free space, inside the status bar's worktree popover. Loaded when it opens (measuring walks
 *  gigabytes, the daemon keeps the answer) and again on `disk.changed`; nothing runs while it is closed. */
export function DiskPanel(props: { onClose?: () => void }) {
  // undefined: loading; null: a daemon without `disk.status`
  const [st, setSt] = createSignal<DiskStatus | null | undefined>(undefined);
  const [busy, setBusy] = createSignal<string | null>(null);
  const load = (refresh = false) =>
    diskStatus(refresh).then(setSt, (e) => {
      console.debug("disk.status", e);
      setSt((s) => s ?? null);
    });
  onMount(() => {
    const live = createLive(() => load(), ["disk.changed"], { debounceMs: 1000 });
    live.now();
    onCleanup(live.stop);
  });

  const intoOf = (w: DiskWorktree) => projects().find((p) => p.id === w.project)?.default_branch ?? "main";

  /** dry run first, so the question names what it frees; `w` absent: every inactive worktree */
  const clean = async (w?: DiskWorktree) => {
    if (busy()) return;
    setBusy(w?.path ?? "*");
    try {
      const o = { path: w?.path, force: true };
      const dry = await sweepArtifacts({ ...o, dry_run: true });
      if (!dry.freed_mb) {
        const why = dry.skipped[0];
        toast({
          text: w && why ? coded(why.code ?? undefined, why.params, why.text ?? D.result.nothing) : D.result.nothing,
          kind: "info",
          key: TOAST_KEY,
        });
        return;
      }
      const size = fmtMb(dry.freed_mb);
      const ok = await ask(w ? D.ask.one(wtName(w), size) : D.ask.all(dry.artifacts.length, size), {
        title: D.ask.title,
        kind: "warning",
        okLabel: D.ask.ok,
      });
      if (!ok) return;
      const r = await sweepArtifacts(o);
      const fail = r.skipped.find((x) => x.code === "artifacts.failed");
      if (fail)
        toast({
          text: coded("artifacts.failed", fail.params, fail.text ?? ""),
          kind: "warn",
          key: `${TOAST_KEY}:failed`,
        });
      toast({ text: r.freed_mb ? D.result.freed(fmtMb(r.freed_mb)) : D.result.nothing, kind: "info", key: TOAST_KEY });
    } catch (e) {
      toast({ text: D.result.failed(errorText(e)), kind: "warn", key: `${TOAST_KEY}:failed` });
    } finally {
      setBusy(null);
      void load();
    }
  };

  const integrate = (w: DiskWorktree) =>
    void integrateFlow({
      projectId: w.project,
      path: w.path,
      name: wtName(w),
      into: intoOf(w),
      ahead: w.ahead,
      dirty: !!pendingOf(overview(), w.project, w.path)?.dirty,
    }).then(() => load());

  const open = (w: DiskWorktree) => {
    selectWorktree(w.path);
    props.onClose?.();
  };

  return (
    <Show when={st() !== null}>
      <div class="sb-head sb-disk-head">
        <span>{D.panel.head}</span>
        <Button variant="ghost" size="chip" disabled={st() === undefined} onClick={() => void load(true)}>
          {D.panel.measure}
        </Button>
      </div>
      <Show when={st()} fallback={<div class="sb-disk-note lim-dim">{D.panel.loading}</div>}>
        {(s) => (
          <>
            <ul class="sb-merged">
              <For each={s().volumes}>
                {(v) => (
                  <li classList={{ "sb-disk-low": v.low }}>
                    <span>{D.panel.volume(v.root, fmtMb(v.free_mb), fmtMb(v.total_mb))}</span>
                    <Show when={v.low}>
                      <span>{D.panel.low}</span>
                    </Show>
                  </li>
                )}
              </For>
            </ul>
            <Show when={staleList(s(), Date.now()).length}>
              <div class="sb-head">{D.stale.head}</div>
              <ul class="sb-disk-stale">
                <For each={staleList(s(), Date.now())}>
                  {(w) => (
                    <li>
                      <div>
                        {D.stale.line(
                          w.branch || wtName(w),
                          w.ahead,
                          intoOf(w),
                          ago(idleSecs(w, Date.now())),
                          fmtMb(w.total_mb),
                        )}
                      </div>
                      <div class="sb-disk-actions">
                        <Button variant="default" size="chip" disabled={!!busy()} onClick={() => integrate(w)}>
                          {D.stale.integrate}
                        </Button>
                        <Button
                          variant="chip"
                          size="chip"
                          disabled={!!busy() || !w.reclaimable_mb}
                          title={D.panel.cleanTip}
                          onClick={() => void clean(w)}
                        >
                          {busy() === w.path ? D.panel.cleaning : D.panel.clean}
                        </Button>
                        <Button variant="chip" size="chip" onClick={() => open(w)}>
                          {D.stale.open}
                        </Button>
                      </div>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
            <Show when={bySize(s()).length} fallback={<div class="sb-disk-note lim-dim">{D.panel.empty}</div>}>
              <ul class="sb-merged sb-disk-list">
                <For each={bySize(s())}>
                  {(w) => (
                    <li
                      title={D.panel.sizeTip(
                        fmtMb(w.target_mb),
                        fmtMb(w.node_modules_mb),
                        fmtMb(w.cache_mb),
                        fmtMb(w.reclaimable_mb),
                      )}
                    >
                      <span class="sb-name">{wtName(w)}</span>
                      <span class="sb-disk-size">
                        <Show when={!inactive(w)}>
                          <span class="lim-dim">{D.panel.active}</span>
                        </Show>
                        <span>{fmtMb(w.total_mb)}</span>
                        <Button
                          variant="chip"
                          size="chip"
                          disabled={!!busy() || !inactive(w) || !w.reclaimable_mb}
                          title={D.panel.cleanTip}
                          aria-label={`${D.panel.clean}: ${wtName(w)}`}
                          onClick={() => void clean(w)}
                        >
                          {busy() === w.path ? D.panel.cleaning : D.panel.clean}
                        </Button>
                      </span>
                    </li>
                  )}
                </For>
              </ul>
              <Button
                variant="outline"
                size="sm"
                class="mx-1.5 mb-0.5 mt-1"
                disabled={!!busy() || !reclaimableTotal(s())}
                title={D.panel.cleanAllTip}
                onClick={() => void clean()}
              >
                {busy() === "*" ? D.panel.cleaning : `${D.panel.cleanAll} (${fmtMb(reclaimableTotal(s()))})`}
              </Button>
            </Show>
          </>
        )}
      </Show>
    </Show>
  );
}
