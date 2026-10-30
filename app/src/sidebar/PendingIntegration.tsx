import { Show, createSignal } from "solid-js";
import { Icon } from "../Icon";
import { Button } from "../ui";
import { integrateUndo, integrateWorktree, type IntegrateResult } from "../api";
import { ask } from "../Confirm";
import { errorText } from "../appError";
import { coded } from "../msg";
import { toast } from "../toasts";
import { P } from "../strings.panels";
import { overview, refreshOverview, refreshProjects, refreshSessions, sessionsOf } from "../store";
import { pendingOf, pendingState } from "../statusGroups";

const T = P.sidebar.pending;
const F = P.flows;

/** how long the "Desfazer" toast of an integration stays (it also pauses while hovered or focused) */
const UNDO_TTL_MS = 20000;

/** The pending entry of a worktree that has no running agent: it is waiting to be integrated, never "active". */
export const pendingFor = (projectId: string, path: string) =>
  sessionsOf(path).some((s) => s.running) ? undefined : pendingOf(overview(), projectId, path);

const refreshAll = async () => {
  await refreshOverview();
  await refreshProjects();
  await refreshSessions();
};

type Target = { projectId: string; path: string; name: string; into: string };

/** takes the integration back (the daemon refuses when the default branch moved or was pushed) */
const undo = async (t: Target, r: IntegrateResult) => {
  if (!r.default_branch || !r.prev_tip || !r.new_tip || !r.branch || !r.branch_tip) return;
  try {
    await integrateUndo({
      project: t.projectId,
      default_branch: r.default_branch,
      prev_tip: r.prev_tip,
      new_tip: r.new_tip,
      branch: r.branch,
      branch_tip: r.branch_tip,
    });
    toast({ text: F.integrateUndone(t.name), kind: "info", key: `integrate:${t.path}` });
  } catch (e) {
    toast({
      text: F.integrateUndoFailed(t.name, errorText(e)),
      kind: "warn",
      key: `integrate:${t.path}`,
      sticky: true,
    });
  }
  await refreshAll();
};

/** Asks, integrates the worktree into `into` and toasts the outcome (with "Desfazer" when it can be taken back). Also used
 *  by the disk panel's warning about unintegrated work. */
export const integrateFlow = async (t: Target & { ahead: number; dirty: boolean }) => {
  const ok = await ask(F.integrateAsk(t.name, t.ahead, t.into, t.dirty), {
    title: F.integrateTitle,
    kind: "warning",
    okLabel: F.integrateOk,
  });
  if (!ok) return;
  try {
    const r = await integrateWorktree(t.projectId, t.path);
    const canUndo =
      r.integrated && r.pushed !== true && !!(r.default_branch && r.prev_tip && r.new_tip && r.branch && r.branch_tip);
    toast({
      text: !r.integrated ? T.nothing(t.name) : r.removed ? T.doneRemoved(t.name, r.into) : T.done(t.name, r.into),
      kind: "info",
      key: `integrate:${t.path}`,
      ...(canUndo ? { action: { label: F.undo, run: () => void undo(t, r) }, ttl: UNDO_TTL_MS } : {}),
    });
  } catch (e) {
    toast({ text: T.failed(t.name, errorText(e)), kind: "warn", key: `integrate:${t.path}`, sticky: true });
  }
  await refreshAll();
};

/** "Por integrar" block under a worktree row: why it waits, and the three ways out. */
export function PendingIntegration(props: {
  projectId: string;
  path: string;
  name: string;
  into: string;
  onStartLead: () => void;
  onDrop: () => void;
}) {
  const [busy, setBusy] = createSignal(false);
  const p = () => pendingFor(props.projectId, props.path);
  const state = () => {
    const x = p();
    return x ? pendingState(x) : "pending";
  };
  const reason = () => {
    const a = p()?.attention;
    return a ? coded(a.code, a.params, a.text) : "";
  };
  const tip = () => reason() || T.tip(p()?.ahead ?? 0, props.into);

  const integrate = async () => {
    if (busy()) return;
    const x = p();
    setBusy(true);
    try {
      await integrateFlow({
        projectId: props.projectId,
        path: props.path,
        name: props.name,
        into: props.into,
        ahead: x?.ahead ?? 0,
        dirty: !!x?.dirty,
      });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Show when={p()}>
      {(x) => (
        <div class="wt-pending" classList={{ attention: state() === "attention" }} data-state={state()} title={tip()}>
          <div class="wt-pending-head">
            <Icon name={state() === "attention" ? "warning" : "arrowUp"} size={12} />
            <strong>{state() === "attention" ? T.attentionLabel : T.label}</strong>
            <span class="wt-pending-meta">
              {F.ahead(x().ahead)} {x().dirty ? `· ${T.dirty}` : ""}
            </span>
          </div>
          <Show when={reason()}>
            <div class="wt-pending-reason">{reason()}</div>
          </Show>
          <div class="wt-pending-actions">
            <Button
              type="button"
              variant="default"
              size="chip"
              disabled={busy()}
              title={T.integrateTip}
              onClick={() => void integrate()}
            >
              {busy() ? T.integrating : T.integrate}
            </Button>
            <Button type="button" variant="chip" size="chip" onClick={props.onStartLead}>
              {T.openMain}
            </Button>
            <Button type="button" variant="chip" size="chip" onClick={props.onDrop}>
              {T.discard}
            </Button>
          </div>
        </div>
      )}
    </Show>
  );
}
