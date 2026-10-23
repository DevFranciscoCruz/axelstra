import { openUrl } from "@tauri-apps/plugin-opener";
import { Badge, Button, Checkbox } from "./ui";
import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { press } from "./a11y";
import { ask } from "./Confirm";
import { Icon } from "./Icon";
import { CheckItem, ShipStatus, gitChecks, gitMerge } from "./api";
import { push, shipEvents } from "./notifications";
import { CHECK_LABEL, CI_LABEL, REVIEW_LABEL, SHIP_POLL_MS, autoMergeQuestion, canAutoMerge, mergeLabel, mergeQuestion, problemText, shouldPoll } from "./shipLogic";
import { findWorktree, selectedWorktree } from "./store";
import "./ship.css";

/** last state read per worktree, so a change is noticed even after looking at another worktree in between */
const lastSeen = new Map<string, ShipStatus>();

const failed = (e: unknown): ShipStatus => ({
  problem: "error", detail: String(e), number: 0, title: "", url: "", pr_state: "open", draft: false, ci: "none", checks: [], review: "", mergeable: "unknown", merge_state: "", can_merge: false,
});

/** CI, review and merge state of the pull request behind the selected worktree's branch, with the merge buttons.
 *  Read every 20 s, only while this is on screen and the window has focus. */
export function ShipPanel() {
  const cur = () => findWorktree(selectedWorktree());
  const branch = () => cur()?.wt.branch ?? "";
  const [st, setSt] = createSignal<ShipStatus | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [open, setOpen] = createSignal(true);
  const [delBranch, setDelBranch] = createSignal(false);
  const [note, setNote] = createSignal<{ ok: boolean; text: string } | null>(null);
  let focused = true;
  let inflight = false;

  const load = async () => {
    const path = selectedWorktree();
    const c = cur();
    // the main worktree sits on the default branch: there is no PR of its own to show
    if (!path || !c || c.wt.is_main || !c.wt.branch || inflight) return;
    inflight = true;
    try {
      let s: ShipStatus;
      try {
        s = await gitChecks(path);
      } catch (e) {
        s = failed(e);
      }
      if (selectedWorktree() !== path) return;
      const name = c.wt.branch ?? path;
      for (const ev of shipEvents(lastSeen.get(path) ?? null, s, path, name)) push(ev);
      if (s.problem) lastSeen.delete(path);
      else lastSeen.set(path, s);
      setSt(s);
    } finally {
      inflight = false;
    }
  };

  createEffect(() => {
    selectedWorktree();
    setSt(null);
    setNote(null);
    void load();
  });

  onMount(() => {
    const tick = () => shouldPoll(document.hidden, focused) && void load();
    const timer = setInterval(tick, SHIP_POLL_MS);
    const onFocus = () => {
      focused = true;
      tick();
    };
    const onBlur = () => (focused = false);
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
    document.addEventListener("visibilitychange", tick);
    onCleanup(() => {
      clearInterval(timer);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
      document.removeEventListener("visibilitychange", tick);
    });
  });

  const openLink = async (url: string) => {
    try {
      await openUrl(url);
    } catch (e) {
      setNote({ ok: false, text: String(e) });
    }
  };

  const merge = async (auto: boolean) => {
    const s = st();
    const path = selectedWorktree();
    if (!s || !path || busy()) return;
    const q = auto ? autoMergeQuestion(s) : mergeQuestion(s, branch(), delBranch());
    // merging publishes to the remote and cannot be undone: always confirm, with the PR title in the question
    if (!(await ask(q, { title: auto ? "Auto-merge" : "Fazer merge", kind: "warning", okLabel: auto ? "Ativar auto-merge" : "Fazer merge" }))) return;
    setBusy(true);
    setNote(null);
    try {
      await gitMerge(path, auto, !auto && delBranch());
      setNote({ ok: true, text: auto ? "Auto-merge ativado" : "Merge feito" });
    } catch (e) {
      setNote({ ok: false, text: String(e) });
      push({ kind: "error", worktree: path, text: `Falhou o ${auto ? "auto-merge" : "merge"} de #${s.number}: ${String(e).slice(0, 160)}` });
    } finally {
      setBusy(false);
      void load();
    }
  };

  return (
    <Show when={cur() && !cur()!.wt.is_main}>
      <section class="ship" aria-label="Estado do pull request">
        <Show when={st()} fallback={<div class="ship-line muted">A ver o pull request…</div>}>
          {(s) => (
            <Show
              when={!s().problem}
              fallback={
                <div class="ship-problem" classList={{ soft: s().problem === "nopr" }}>
                  <Show when={s().problem !== "nopr"}>
                    <span class="ship-icon" aria-hidden="true"><Icon name="warning" size={13} /></span>
                  </Show>
                  <div>
                    <div>{problemText(s()).text}</div>
                    <div class="muted">{problemText(s()).fix}</div>
                  </div>
                </div>
              }
            >
              <div class="sc-title" {...press(() => setOpen(!open()))} aria-expanded={open()}>
                <span class="chev"><Icon name={open() ? "chevDown" : "chevRight"} /></span>
                <span>PULL REQUEST #{s().number}</span>
                <span class="grow" />
                <span class={`ship-ci ${s().ci}`}>
                  <Show when={CI_LABEL[s().ci][0]}>
                    <Icon name={CI_LABEL[s().ci][0]} size={12} />
                  </Show>{" "}
                  {CI_LABEL[s().ci][1]}
                </span>
              </div>
              <div class="ship-title" title={s().title}>{s().title}</div>
              <div class="ship-facts">
                <Badge>{REVIEW_LABEL[s().review]}</Badge>
                <Badge variant={s().mergeable === "no" ? "bad" : "default"}>{mergeLabel(s())}</Badge>
              </div>
              <Show when={open() && s().checks.length}>
                <ul class="ship-checks" aria-label="Checks">
                  <For each={s().checks}>
                    {(c: CheckItem) => (
                      <li class={`ship-check ${c.state}`}>
                        <span class="ship-mark">
                          <Show when={CHECK_LABEL[c.state][0]}>
                            <Icon name={CHECK_LABEL[c.state][0]} size={11} />
                          </Show>
                        </span>
                        <Show when={c.link} fallback={<span class="ship-name" title={c.name}>{c.name}</span>}>
                          <Button variant="row" size="bare" class="ship-name block truncate" title={`Abrir ${c.name} no browser`} onClick={() => void openLink(c.link)}>{c.name}</Button>
                        </Show>
                        <span class="ship-state">{CHECK_LABEL[c.state][1]}</span>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
              <Show when={s().pr_state === "merged"}>
                <div class="ship-line">Este worktree já foi integrado: o janitor pode removê-lo.</div>
              </Show>
              <div class="ship-actions">
                <Button size="sm" disabled={!s().url} onClick={() => void openLink(s().url)}>Abrir no browser</Button>
                <Show when={canAutoMerge(s())}>
                  <Button size="sm" disabled={busy()} onClick={() => void merge(true)}>Ativar auto-merge</Button>
                </Show>
                <Show when={s().can_merge}>
                  <Button variant="default" size="sm" disabled={busy()} onClick={() => void merge(false)}>Fazer merge</Button>
                </Show>
              </div>
              <Show when={s().can_merge}>
                <Checkbox labelClass="ship-opt" checked={delBranch()} onChange={(e) => setDelBranch(e.currentTarget.checked)} label="Apagar o branch depois do merge" />
              </Show>
            </Show>
          )}
        </Show>
        <div class="pnote" role="status" aria-live="polite" classList={{ bad: note() ? !note()!.ok : false }}>
          {note()?.text ?? ""}
        </div>
      </section>
    </Show>
  );
}
