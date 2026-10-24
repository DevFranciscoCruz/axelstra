import { lazyPair } from "./i18n";
import { Loading } from "./Loading";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Badge, Button } from "./ui";
import { Icon } from "./Icon";
import { For, Show, createResource, createSignal } from "solid-js";
import { GhItem, ReviewComment, ghComments, ghList, ghSendComment, worktreeFromPr } from "./api";
import {
  SendStep,
  commentHints,
  confirmKey,
  displayBody,
  isNoLeadError,
  locationLabel,
  reviewLine,
  threads,
  worktreeForPr,
} from "./prComments";
import "./pulls.css";
import { RightTabs } from "./RightTabs";
import { P } from "./strings.panels";
import {
  findWorktree,
  refreshProjects,
  selectIfStill,
  navStamp,
  selectedWorktree,
  setError,
  rightTab,
  showNotice,
} from "./store";

const CHECK: Record<string, [string, string]> = {
  pass: lazyPair("check", () => P.pulls.checks.pass),
  fail: lazyPair("x", () => P.pulls.checks.fail),
  pending: lazyPair("ellipsis", () => P.pulls.checks.pending),
};
const REVIEW = P.pulls.review;
const C = P.pulls.comments;

/** The review comments of one PR, read when the list is opened. Sending one to the agent shows the exact text first. */
function ReviewComments(props: { item: GhItem; project: string; worktree: string | null }) {
  const [open, setOpen] = createSignal(false);
  const [data, { refetch }] = createResource(open, async (o) =>
    o ? ghComments(props.project, props.item.number) : null,
  );
  const [send, setSend] = createSignal<SendStep>({ step: "idle" });
  const n = () => props.item.number;
  const listId = () => `prc-${n()}`;
  // the comment the send flow is on, if any
  const active = () => {
    const s = send();
    return s.step === "idle" ? null : s.comment;
  };
  const confirmText = () => {
    const s = send();
    return s.step === "confirm" || s.step === "sending" ? s.text : "";
  };

  const fail = (e: unknown) => {
    setError(isNoLeadError(e) ? C.noLead : String(e));
    setSend({ step: "idle" });
  };
  // the text comes from the daemon, built from GitHub's current version of the comment: it is what will be delivered
  const begin = async (c: ReviewComment) => {
    const wt = props.worktree;
    if (!wt || send().step !== "idle") return;
    setSend({ step: "loading", comment: c.id });
    try {
      const r = await ghSendComment(wt, props.project, n(), c.id, true);
      setSend({ step: "confirm", comment: c.id, text: r.text });
    } catch (e) {
      fail(e);
    }
  };
  const confirm = async () => {
    const s = send();
    const wt = props.worktree;
    if (s.step !== "confirm" || !wt) return;
    setSend({ step: "sending", comment: s.comment, text: s.text });
    try {
      await ghSendComment(wt, props.project, n(), s.comment, false);
      showNotice(C.sent(n()));
      setSend({ step: "idle" });
    } catch (e) {
      fail(e);
    }
  };
  const key = (e: KeyboardEvent) => {
    // a focused button keeps its own Enter
    if ((e.target as HTMLElement).tagName === "BUTTON") return;
    const a = confirmKey(send(), e.key);
    if (!a) return;
    e.preventDefault();
    e.stopPropagation();
    if (a === "send") void confirm();
    else setSend({ step: "idle" });
  };

  const Row = (p: { c: ReviewComment; reply?: boolean }) => (
    <li class="prc" classList={{ reply: p.reply }}>
      <div class="prc-head">
        <span class="prc-author">{p.c.author}</span>
        <span class="prc-loc mono" title={locationLabel(p.c)}>
          {locationLabel(p.c)}
        </span>
        <For each={commentHints(p.c)}>{(h) => <Badge>{h}</Badge>}</For>
        <span class="grow" />
        <Button
          size="sm"
          variant="outline"
          disabled={!props.worktree || send().step !== "idle"}
          title={props.worktree ? C.sendTip : C.noWorktree}
          aria-label={`${C.send}: ${p.c.author}, ${locationLabel(p.c)}`}
          onClick={() => void begin(p.c)}
        >
          {send().step === "loading" && active() === p.c.id ? "…" : C.send}
        </Button>
      </div>
      <div class="prc-body">{displayBody(p.c.body)}</div>
      <Show when={(send().step === "confirm" || send().step === "sending") && active() === p.c.id}>
        <div
          class="prc-confirm"
          role="group"
          aria-label={C.confirmTitle}
          tabindex="-1"
          onKeyDown={key}
          ref={(el) => queueMicrotask(() => el.focus())}
        >
          <div class="muted">{C.confirmIntro}</div>
          <pre class="prc-text" tabindex="0" aria-label={C.confirmTitle}>
            {confirmText()}
          </pre>
          <div class="prc-actions">
            <Button size="sm" variant="default" disabled={send().step === "sending"} onClick={() => void confirm()}>
              {C.confirmOk}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={send().step === "sending"}
              onClick={() => setSend({ step: "idle" })}
            >
              {C.confirmCancel}
            </Button>
            <span class="muted">{C.confirmHint}</span>
          </div>
        </div>
      </Show>
    </li>
  );

  return (
    <div class="prc-wrap">
      <Button
        variant="link"
        size="bare"
        aria-expanded={open()}
        aria-controls={listId()}
        title={C.toggleTip}
        onClick={() => setOpen(!open())}
      >
        <Icon name={open() ? "chevDown" : "chevRight"} size={11} /> {C.toggle}
      </Button>
      <Show when={open()}>
        <div id={listId()} role="region" aria-label={C.list(n())}>
          <Show when={!data.loading} fallback={<div class="muted prc-note">{C.loading}</div>}>
            <Show
              when={!data.error}
              fallback={
                <div class="placeholder">
                  {String(data.error?.message ?? data.error)}
                  <Button size="sm" onClick={() => void refetch()}>
                    {P.pulls.refresh}
                  </Button>
                </div>
              }
            >
              <Show when={data()}>
                {(d) => (
                  <>
                    <Show when={!props.worktree}>
                      <div class="muted prc-note">{C.noWorktree}</div>
                    </Show>
                    <Show when={d().reviews.length}>
                      <ul class="prc-list" aria-label={C.reviewsHead}>
                        <For each={d().reviews}>
                          {(r) => (
                            <li class="prc">
                              <div class="prc-head">
                                <span class="prc-author">{reviewLine(r)}</span>
                              </div>
                              <div class="prc-body">{displayBody(r.body)}</div>
                            </li>
                          )}
                        </For>
                      </ul>
                    </Show>
                    <ul class="prc-list" aria-label={C.list(n())}>
                      <For
                        each={threads(d().comments)}
                        fallback={
                          <Show when={!d().reviews.length}>
                            <li class="muted prc-note">{C.none}</li>
                          </Show>
                        }
                      >
                        {(t) => (
                          <>
                            <Row c={t.root} />
                            <For each={t.replies}>{(r) => <Row c={r} reply />}</For>
                          </>
                        )}
                      </For>
                    </ul>
                  </>
                )}
              </Show>
            </Show>
          </Show>
        </div>
      </Show>
    </div>
  );
}

/** Open pull requests and issues of the selected project, read through the GitHub CLI. */
export function PullsPanel() {
  const [kind, setKind] = createSignal<"pr" | "issue">("pr");
  const project = () => findWorktree(selectedWorktree())?.project.id ?? null;
  const worktrees = () => findWorktree(selectedWorktree())?.project.worktrees ?? [];
  const [items, { refetch }] = createResource(
    () => ({ p: project(), k: kind() }),
    async ({ p, k }): Promise<GhItem[]> => (p ? ghList(p, k) : []),
  );
  const [busy, setBusy] = createSignal<number | null>(null);

  const open = async (url: string) => {
    try {
      await openUrl(url);
    } catch (e) {
      setError(String(e));
    }
  };

  // a worktree holding the PR's head, ready for an agent to review or fix it
  const work = async (n: number) => {
    const p = project();
    if (!p) return;
    setBusy(n);
    try {
      const stamp = navStamp();
      const w = await worktreeFromPr(p, n);
      await refreshProjects();
      selectIfStill(stamp, w.path);
      showNotice(P.pulls.wtCreated(n));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <aside class="rightbar" id="rpanel" role="tabpanel" aria-labelledby={`rtab-${rightTab()}`}>
      <RightTabs />
      <div class="rtools">
        <Button
          variant="icon"
          size="icon"
          class={kind() === "pr" ? "text-primary" : undefined}
          aria-pressed={kind() === "pr"}
          onClick={() => setKind("pr")}
        >
          {P.pulls.prs}
        </Button>
        <Button
          variant="icon"
          size="icon"
          class={kind() === "issue" ? "text-primary" : undefined}
          aria-pressed={kind() === "issue"}
          onClick={() => setKind("issue")}
        >
          {P.pulls.issues}
        </Button>
        <span class="grow" />
        <Button
          variant="icon"
          size="icon"
          title={P.pulls.refresh}
          aria-label={P.pulls.refresh}
          onClick={() => void refetch()}
        >
          <Icon name="refresh" />
        </Button>
      </div>
      <div class="rlist">
        <Show when={!items.loading} fallback={<Loading onRetry={() => void refetch()} />}>
          <Show
            when={!items.error}
            fallback={<div class="placeholder">{String(items.error?.message ?? items.error)}</div>}
          >
            <For each={items()} fallback={<div class="placeholder">{P.pulls.nothing}</div>}>
              {(i) => (
                <div class="pr">
                  <div class="pr-title">
                    <Show when={i.checks}>
                      <span class={`chk ${i.checks}`} title={CHECK[i.checks][1]}>
                        <Icon name={CHECK[i.checks][0]} size={12} />
                      </span>
                    </Show>
                    <span class="pr-num">#{i.number}</span>
                    <span class="pr-text" title={i.title}>
                      {i.title}
                    </span>
                  </div>
                  <div class="pr-meta">
                    <span>{i.author}</span>
                    <Show when={i.draft}>
                      <Badge>{P.pulls.draft}</Badge>
                    </Show>
                    <Show when={i.review}>
                      <Badge>{REVIEW[i.review]}</Badge>
                    </Show>
                    <Show when={i.branch}>
                      <Badge class="mono">{i.branch}</Badge>
                    </Show>
                    <span class="grow" />
                    <Show when={kind() === "pr"}>
                      <Button
                        variant="icon"
                        size="icon"
                        disabled={busy() === i.number}
                        title={P.pulls.openWtTip}
                        onClick={() => void work(i.number)}
                      >
                        {busy() === i.number ? "…" : P.pulls.worktree}
                      </Button>
                    </Show>
                    <Button
                      variant="icon"
                      size="icon"
                      title={P.pulls.openBrowser}
                      aria-label={P.pulls.openBrowser}
                      onClick={() => void open(i.url)}
                    >
                      <Icon name="external" />
                    </Button>
                  </div>
                  <Show when={kind() === "pr" && project()}>
                    <ReviewComments item={i} project={project()!} worktree={worktreeForPr(i, worktrees())} />
                  </Show>
                </div>
              )}
            </For>
          </Show>
        </Show>
      </div>
    </aside>
  );
}
