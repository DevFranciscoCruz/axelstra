import { For, Show, createEffect, createSignal, onCleanup } from "solid-js";
import { AgentHit, agentSearch, geminiPathVariants, hitResume, launchDefaultAgent } from "./api";
import { moveIndex } from "./a11y";
import { ago } from "./time";
import { Modal } from "./Modal";
import { findWorktree, projects, projectTerse, refreshSessions, setError, setShowAgentSearch, selectWorktree, navStamp, openIfStill, sessionsOf, setCreatingWorktree, toast } from "./store";
import { allowManualAgent } from "./sidebar/mainRules";
import { Input } from "./ui";
import { P } from "./strings.panels";

const sha256Hex = async (s: string) =>
  [...new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(s)))].map((b) => b.toString(16).padStart(2, "0")).join("");
const baseName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;
/** full-text search over past Claude/Codex/Gemini conversations; a hit in a known worktree reopens that conversation (Codex ones cannot be reopened) */
export function AgentSearch() {
  const [q, setQ] = createSignal("");
  const [hits, setHits] = createSignal<AgentHit[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [done, setDone] = createSignal(false);
  const [sel, setSel] = createSignal(0);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  onCleanup(() => clearTimeout(timer));
  // Gemini logs name a project by the hash of its path: hash every known worktree to tell which one a hit belongs to
  const [gemWt, setGemWt] = createSignal<Record<string, string>>({});
  void (async () => {
    const m: Record<string, string> = {};
    try {
      for (const p of projects()) for (const w of p.worktrees) for (const v of geminiPathVariants(w.path)) m[await sha256Hex(v)] = w.path;
    } catch {
      // no crypto in this webview: Gemini hits just show as unknown folders
    }
    setGemWt(m);
  })();

  const run = (text: string) => {
    clearTimeout(timer);
    setDone(false);
    if (text.trim().length < 2) return setHits([]);
    timer = setTimeout(async () => {
      const mine = ++seq;
      setBusy(true);
      try {
        const r = await agentSearch(text.trim());
        if (mine === seq) {
          setHits(r);
          setDone(true);
        }
      } catch (e) {
        if (mine === seq) setError(String(e).includes("unknown method") ? P.agentSearch.restartDaemon : String(e));
      } finally {
        if (mine === seq) setBusy(false);
      }
    }, 300);
  };

  const folder = (h: AgentHit) => (h.agent === "gemini" ? (h.project && gemWt()[h.project]) || "" : h.cwd);
  const target = (h: AgentHit) => (hitResume(h) ? findWorktree(folder(h)) : null);
  const open = async (h: AgentHit) => {
    const t = target(h);
    if (!t) return;
    setShowAgentSearch(false);
    // resuming into the main checkout beside its running main would make a task there: offer a worktree instead
    const deps = {
      notify: (text: string, action: { label: string; run: () => void }) => toast({ text, kind: "info", action }),
      newWorktree: (project: string) => setCreatingWorktree(project),
    };
    if (!allowManualAgent(t, sessionsOf(t.wt.path), deps)) return;
    selectWorktree(t.wt.path);
    const stamp = navStamp();
    try {
      const id = await launchDefaultAgent(t.project.id, t.wt.path, h.agent, h.agent, { resume: hitResume(h) ?? undefined }, undefined, projectTerse(t.project.id));
      await refreshSessions();
      // the resumed agent is what the user picked: open exactly it, unless they went elsewhere meanwhile
      openIfStill(stamp, t.wt.path, id);
    } catch (e) {
      setError(String(e));
    }
  };

  createEffect(() => {
    hits();
    setSel(0);
  });
  // keep the highlighted hit visible while arrowing through a long list
  createEffect(() => {
    sel();
    queueMicrotask(() => document.querySelector(".search-hit.on")?.scrollIntoView({ block: "nearest" }));
  });
  const onKey = (e: KeyboardEvent) => {
    // Home and End stay with the text field (cursor)
    const to = e.key.startsWith("Arrow") ? moveIndex(sel(), hits().length, e.key) : null;
    if (to !== null) {
      e.preventDefault();
      setSel(to);
    } else if (e.key === "Enter" && hits()[sel()]) {
      e.preventDefault();
      void open(hits()[sel()]);
    }
  };

  return (
    <Modal label={P.agentSearch.title} onClose={() => setShowAgentSearch(false)} class="modal auto">
      <Input
        class="m-title"
        placeholder={P.agentSearch.placeholder}
        role="combobox"
        aria-expanded={hits().length > 0}
        aria-controls="search-list"
        aria-activedescendant={hits().length ? `search-hit-${sel()}` : undefined}
        aria-label={P.agentSearch.label}
        value={q()}
        onKeyDown={onKey}
        onInput={(e) => {
          setQ(e.currentTarget.value);
          run(e.currentTarget.value);
        }}
        ref={(el) => queueMicrotask(() => el.focus())}
      />
      <div class="auto-list" id="search-list" role="listbox" aria-label={P.agentSearch.found}>
        <For each={hits()}>
          {(h, i) => (
            <div
              class="auto-item search-hit"
              id={`search-hit-${i()}`}
              role="option"
              aria-selected={i() === sel()}
              aria-disabled={!target(h)}
              classList={{ off: !target(h), on: i() === sel() }}
              onMouseEnter={() => setSel(i())}
              onClick={() => void open(h)}
              title={target(h) ? P.agentSearch.resume : folder(h) || h.cwd}
            >
              <div class="auto-main">
                <div class="auto-name">
                  {h.agent} · {baseName(folder(h) || "?")} <span class="auto-note">{ago(Date.now() / 1000 - h.ts)}</span>
                </div>
                <div class="auto-note">{h.snippet}</div>
              </div>
            </div>
          )}
        </For>
      </div>
      <div class="auto-note" role="status">
        <Show when={busy()}>{P.agentSearch.searching}</Show>
        <Show when={!busy() && done() && hits().length === 0}>{P.agentSearch.none}</Show>
      </div>
      <div class="auto-note">{P.agentSearch.note}</div>
    </Modal>
  );
}
