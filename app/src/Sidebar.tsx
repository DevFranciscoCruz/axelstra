import { open } from "@tauri-apps/plugin-dialog";
import { ask } from "./Confirm";
import { For, Show, createResource, createSignal } from "solid-js";
import { addProject, projectLogo, createWorktree, launchDefaultAgent, openInEditor, setProjectSetup, defaultShell, spawnSession, writeSession, removeProject, removeWorktree, killSession, removeSession } from "./api";
import { Modal } from "./Modal";
import { MenuItem, showMenu, showMenuAt } from "./ContextMenu";
import { invoke } from "@tauri-apps/api/core";
import { press } from "./a11y";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import logo from "./assets/logo.png";
import {
  AGENTS,
  isPinned,
  togglePin,
  setAttention,
  setShowAutomations,
  attention,
  sessionState,
  wtStatus,
  prByBranch,
  prKey,
  defaultAgent,
  pickAgent,
  setDefaultAgent,
  confirmDiscard,
  editorDirty,
  flowState,
  setEditorDirty,
  norm,
  projects,
  refreshProjects,
  refreshSessions,
  selectWorktree,
  selectedWorktree,
  sessionsOf,
  setError,
} from "./store";

function ProjectIcon(props: { id: string; name: string; url?: string | null }) {
  const [broken, setBroken] = createSignal(false);
  // a logo file inside the repo wins over the host avatar
  const [repoLogo] = createResource(() => props.id, projectLogo);
  const src = () => repoLogo() ?? props.url;
  return (
    <Show
      when={src() && !broken()}
      fallback={<span class="picon letter">{props.name.slice(0, 1).toUpperCase()}</span>}
    >
      <img class="picon" src={src()!} alt="" onError={() => setBroken(true)} />
    </Show>
  );
}

export function Sidebar() {
  // project id currently showing the "new worktree" input
  const [creating, setCreating] = createSignal<string | null>(null);
  const [newName, setNewName] = createSignal("");
  // agent started in a new worktree: "none" or an AGENTS key, remembered between runs
  const NEWWT_KEY = "axelstra.newwt.agent";
  const [newAgent, setNewAgent] = createSignal<string>(
    (() => {
      try {
        const v = localStorage.getItem(NEWWT_KEY);
        return v === "none" || (v && AGENTS[v]) ? v! : "claude";
      } catch {
        return "claude";
      }
    })(),
  );
  const setNewAgentSig = (v: string) => {
    setNewAgent(v);
    try {
      localStorage.setItem(NEWWT_KEY, v);
    } catch {
      /* preference only */
    }
  };
  // project id whose setup command is being edited
  const [editSetup, setEditSetup] = createSignal<string | null>(null);
  const [setupText, setSetupText] = createSignal("");

  // selecting a worktree never starts anything: agents and terminals come from the user or from restoring sessions
  const openWorktree = async (_projectId: string, path: string, _isMain: boolean) => {
    if (editorDirty()) {
      if (!(await confirmDiscard())) return;
      setEditorDirty(false);
    }
    selectWorktree(path);
  };

  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
    } catch (e) {
      setError(String(e));
    }
    await refreshProjects();
    await refreshSessions();
  };

  const pickProject = () =>
    run(async () => {
      const dir = await open({ directory: true, multiple: false, title: "Escolha um repositório git" });
      if (typeof dir !== "string") return;
      const added = await addProject(dir);
      await refreshProjects();
      const main = projects().find((p) => p.id === added.id)?.worktrees.find((w) => w.is_main);
      if (main) selectWorktree(main.path);
    });

  const [pendingWt, setPendingWt] = createSignal<{ project: string; name: string } | null>(null);
  const submitWorktree = (projectId: string) =>
    run(async () => {
      const name = newName().trim();
      setCreating(null);
      setNewName("");
      if (!name) return;
      // git checks the files out, which takes a while in a big repo: show the row at once
      setPendingWt({ project: projectId, name });
      let w;
      try {
        w = await createWorktree(projectId, name);
      } finally {
        setPendingWt(null);
      }
      selectWorktree(w.path);
      if (newAgent() !== "none") await launchDefaultAgent(projectId, w.path, newAgent(), AGENTS[newAgent()]);
      const setup = projects().find((p) => p.id === projectId)?.setup;
      if (setup) {
        const [program, args] = await defaultShell();
        const { id } = await spawnSession(program, args, "setup", w.path, { project: projectId, worktree: w.path, kind: "shell" });
        // give the shell a moment to print its prompt before typing into it
        setTimeout(() => void writeSession(id, setup + "\r"), 600);
      }
    });

  const saveSetup = (projectId: string) =>
    run(async () => {
      await setProjectSetup(projectId, setupText());
      setEditSetup(null);
    });

  const copy = async (text: string) => {
    try {
      await invoke("clipboard_write", { text });
    } catch (e) {
      setError(String(e));
    }
  };

  // frees the memory of a worktree without deleting it: its terminals and agents are closed
  const sleepWorktree = async (path: string, name: string) => {
    const live = sessionsOf(path).filter((s) => s.running);
    if (live.length > 0) {
      const ok = await ask(`Pôr "${name}" a dormir?\n\nOs ${live.length} terminais e agentes a correr são fechados. O worktree e os ficheiros ficam como estão.`, {
        title: "Dormir",
        kind: "warning",
      });
      if (!ok) return;
    }
    await run(async () => {
      for (const s of sessionsOf(path)) {
        if (s.running) await killSession(s.id);
        await removeSession(s.id);
      }
    });
  };

  const worktreeMenu = (p: { id: string; name: string }, w: { path: string; branch: string | null; is_main: boolean }, label: string): MenuItem[] => [
    { label: "Atualizar", icon: "↻", run: async () => { await refreshProjects(); await refreshSessions(); } },
    { separator: true },
    {
      label: "Abrir em",
      icon: "▭",
      children: [
        { label: "Explorador de ficheiros", run: () => run(() => revealItemInDir(w.path)) },
        { label: "VS Code", run: () => run(() => openInEditor(w.path, "code")) },
      ],
    },
    { label: "Copiar caminho", icon: "⧉", run: () => copy(w.path) },
    { label: "Copiar nome do worktree", icon: "⧉", run: () => copy(w.branch ?? label) },
    { separator: true },
    { label: isPinned(w.path) ? "Desafixar" : "Fixar", icon: "⚲", run: () => togglePin(w.path) },
    { label: "Marcar como não lido", icon: "◔", run: () => setAttention((cur) => new Set(cur).add(norm(w.path))) },
    { separator: true },
    { label: "Dormir", icon: "☾", run: () => sleepWorktree(w.path, label) },
    { label: "Remover worktree", icon: "×", danger: true, disabled: w.is_main, run: () => dropWorktree(p.id, w.path, label) },
  ];

  const dropProject = async (id: string, name: string) => {
    const ok = await ask(`Tirar "${name}" da lista?

Os ficheiros e worktrees não são apagados.`, { title: "Tirar projeto", kind: "warning", okLabel: "Tirar", cancelLabel: "Cancelar" });
    if (ok) await run(() => removeProject(id));
  };

  const projectMenu = (p: { id: string; name: string; path: string; setup?: string | null }): MenuItem[] => [
    { label: "Novo worktree", icon: "+", run: () => setCreating(p.id) },
    { label: "Comando de setup…", icon: "⚙", run: () => { setSetupText(p.setup ?? ""); setEditSetup(p.id); } },
    { separator: true },
    {
      label: "Abrir em",
      icon: "▭",
      children: [
        { label: "Explorador de ficheiros", run: () => run(() => revealItemInDir(p.path)) },
      ],
    },
    { label: "Copiar caminho", icon: "⧉", run: () => copy(p.path) },
    { separator: true },
    { label: "Tirar projeto da lista", icon: "✕", danger: true, run: () => dropProject(p.id, p.name) },
  ];

  const dropWorktree = async (projectId: string, path: string, name: string) => {
    // what would be lost, so the question is not a blind "are you sure"
    const st = wtStatus()[norm(path)];
    const risks: string[] = [];
    if (st?.dirty) risks.push(`${st.dirty} ${st.dirty === 1 ? "ficheiro com alterações" : "ficheiros com alterações"} por guardar`);
    if (st && st.ahead > 0) risks.push(st.upstream ? `${st.ahead} ${st.ahead === 1 ? "commit" : "commits"} por enviar` : `${st.ahead} ${st.ahead === 1 ? "commit" : "commits"} que só existem aqui (sem remoto)`);
    const live = sessionsOf(path).filter((x) => x.running).length;
    if (live) risks.push(`${live} ${live === 1 ? "terminal ou agente a correr" : "terminais ou agentes a correr"}`);
    const lost = risks.length ? `\n\nVais perder: ${risks.join("; ")}.` : "\n\nNão há alterações nem commits por enviar.";
    const ok = await ask(
      `Remover o worktree "${name}"?${lost}\n\nA pasta e o branch são apagados e os terminais dele são fechados.`,
      { title: "Remover worktree", kind: "warning" },
    );
    if (!ok) return;
    await run(async () => {
      for (const s of sessionsOf(path)) {
        if (s.running) await killSession(s.id);
        await removeSession(s.id);
      }
      await removeWorktree(projectId, path);
      if (norm(selectedWorktree()) === norm(path)) selectWorktree(null);
    });
  };

  return (
    <aside class="sidebar">
      <div class="brand">
        <img src={logo} alt="" />
        Axelstra
      </div>
      <button class="primary" onClick={pickProject}>
        + Adicionar projeto
      </button>

      <div class="section">Projetos</div>
      <For each={projects()} fallback={<div class="empty">Nenhum projeto</div>}>
        {(p) => (
          <div class="project">
            <div class="project-head" title={p.path} onContextMenu={(e) => showMenu(e, projectMenu(p))}>
              <ProjectIcon id={p.id} name={p.name} url={p.icon_url} />
              <span class="pname" title={p.slug ?? p.path}>{p.name}</span>
              <button class="icon" title="Novo worktree" onClick={() => setCreating(creating() === p.id ? null : p.id)}>
                +
              </button>
              <button
                class="icon"
                title={p.setup ? `Setup: ${p.setup}` : "Comando de setup para novos worktrees"}
                onClick={() => {
                  setSetupText(p.setup ?? "");
                  setEditSetup(editSetup() === p.id ? null : p.id);
                }}
              >
                ⚙
              </button>
              <button class="icon" title="Tirar da lista (não apaga ficheiros)" aria-label="Tirar projeto da lista" onClick={() => void dropProject(p.id, p.name)}>
                ×
              </button>
            </div>
            <Show when={editSetup() === p.id}>
              <input
                class="wt-input"
                placeholder="setup, ex.: bun install (vazio = nenhum)"
                value={setupText()}
                onInput={(e) => setSetupText(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void saveSetup(p.id);
                  if (e.key === "Escape") setEditSetup(null);
                }}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
            </Show>
            <Show when={pendingWt()?.project === p.id}>
              <div class="item wt pending">
                <span class="dot work" />
                <span class="name">A criar {pendingWt()!.name}…</span>
              </div>
            </Show>
            <For each={[...p.worktrees].sort((a, b) => Number(b.is_main) - Number(a.is_main) || Number(isPinned(b.path)) - Number(isPinned(a.path)))}>
              {(w) => {
                const label = () => w.branch ?? "(detached)";
                const live = () => sessionsOf(w.path).filter((s) => s.running).length;
                const working = () => sessionsOf(w.path).some((s) => sessionState(s) === "working");
                const waiting = () => sessionsOf(w.path).some((s) => sessionState(s) === "waiting");
                const needs = () => attention().has(norm(w.path));
                const queued = () => live() === 0 && flowState()[norm(w.path)] === "queued";
                const st = () => wtStatus()[norm(w.path)];
                const pr = () => prByBranch()[prKey(p.id, w.branch)];
                return (
                  <div
                    class="item wt"
                    classList={{ active: norm(selectedWorktree()) === norm(w.path) }}
                    {...press(() => void openWorktree(p.id, w.path, w.is_main))}
                    onKeyUp={(e) => e.key === "ContextMenu" && showMenuAt(e.currentTarget.getBoundingClientRect().left + 24, e.currentTarget.getBoundingClientRect().bottom, worktreeMenu(p, w, label()))}
                    onContextMenu={(e) => showMenu(e, worktreeMenu(p, w, label()))}
                    title={w.path}
                  >
                    <span
                      class="dot"
                      classList={{ on: live() > 0, work: working() && !waiting(), attn: needs() && !working() && !waiting(), wait: waiting(), queued: queued() }}
                      title={queued() ? "Na fila: o flow ainda não chegou a esta tarefa" : waiting() ? "O agente espera pela tua resposta" : needs() ? "O agente parou: precisa de atenção" : working() ? "A trabalhar" : live() > 0 ? "Parado" : ""}
                    />
                    <span class="name">{w.is_main ? (p.default_branch ?? "main") : label()}</span>
                    <Show when={w.is_main && w.branch !== (p.default_branch ?? "main")}>
                      <span class="off-main" title={`O worktree principal está em "${label()}", não em ${p.default_branch ?? "main"}`}>
                        ⚠
                      </span>
                    </Show>
                    <Show when={st()?.dirty}>
                      <span class="wt-badge" title={`${st()!.dirty} ficheiro(s) com alterações por guardar`}>●{st()!.dirty}</span>
                    </Show>
                    <Show when={st()?.ahead || st()?.behind}>
                      <span class="wt-badge" title={`${st()!.ahead} commit(s) à frente e ${st()!.behind} atrás do remoto`}>
                        {st()!.ahead ? `↑${st()!.ahead}` : ""}
                        {st()!.behind ? `↓${st()!.behind}` : ""}
                      </span>
                    </Show>
                    <Show when={pr()}>
                      <span
                        class="wt-badge pr"
                        classList={{ pass: pr()!.checks === "pass", fail: pr()!.checks === "fail", pend: pr()!.checks === "pending" }}
                        title={`PR #${pr()!.number}: ${pr()!.title}${pr()!.checks ? ` (CI: ${pr()!.checks})` : ""}`}
                      >
                        #{pr()!.number}
                        {pr()!.checks === "pass" ? " ✓" : pr()!.checks === "fail" ? " ✗" : pr()!.checks === "pending" ? " …" : ""}
                      </span>
                    </Show>
                    <Show when={live() > 0}>
                      <span class="count">{live()}</span>
                    </Show>
                    <Show when={isPinned(w.path)}>
                      <span class="pin" title="Fixado">⚲</span>
                    </Show>
                    <button
                      class="x"
                      title="Mais ações"
                      onClick={(e) => {
                        e.stopPropagation();
                        const r = e.currentTarget.getBoundingClientRect();
                        showMenuAt(r.left, r.bottom + 2, worktreeMenu(p, w, label()));
                      }}
                    >
                      ⋯
                    </button>
                    <Show when={!w.is_main}>
                      <button
                        class="x"
                        title="Remover worktree"
                        onClick={(e) => {
                          e.stopPropagation();
                          void dropWorktree(p.id, w.path, label());
                        }}
                      >
                        ×
                      </button>
                    </Show>
                  </div>
                );
              }}
            </For>
          </div>
        )}
      </For>

      <div class="grow" />
      <button class="side-link" onClick={() => setShowAutomations(true)}>
        ⏱ Automatizações
      </button>
      <Show when={creating()}>
        {(pid) => {
          const proj = () => projects().find((x) => x.id === pid());
          return (
            <Modal label="Novo worktree" onClose={() => setCreating(null)}>
              <div class="m-title" style={{ "font-weight": "600" }}>Novo worktree</div>
              <label class="m-full">
                Projeto
                <input value={proj()?.name ?? ""} disabled />
              </label>
              <label class="m-full">
                Nome
                <input
                  placeholder="nome do worktree (também o nome do branch)"
                  value={newName()}
                  onInput={(e) => setNewName(e.currentTarget.value)}
                  onKeyDown={(e) => e.key === "Enter" && newName().trim() && void submitWorktree(pid())}
                  ref={(el) => queueMicrotask(() => el.focus())}
                />
              </label>
              <label class="m-full">
                Agente
                <select value={newAgent()} onChange={(e) => setNewAgentSig(e.currentTarget.value)}>
                  <option value="none">Nenhum (só o worktree)</option>
                  <For each={Object.keys(AGENTS)}>{(a) => <option value={a}>{a}</option>}</For>
                </select>
              </label>
              <div class="m-actions" style={{ "justify-content": "flex-end" }}>
                <button onClick={() => setCreating(null)}>Cancelar</button>
                <button class="primary" disabled={!newName().trim()} onClick={() => void submitWorktree(pid())}>
                  Criar worktree
                </button>
              </div>
            </Modal>
          );
        }}
      </Show>
    </aside>
  );
}
