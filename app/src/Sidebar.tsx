import { open, ask } from "@tauri-apps/plugin-dialog";
import { For, Show, createResource, createSignal } from "solid-js";
import { addProject, projectLogo, createWorktree, launchDefaultAgent, openInEditor, setProjectSetup, defaultShell, spawnSession, writeSession, removeProject, removeWorktree, killSession, removeSession } from "./api";
import { MenuItem, showMenu } from "./ContextMenu";
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
  defaultAgent,
  pickAgent,
  setDefaultAgent,
  error,
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
  const [agentMenu, setAgentMenu] = createSignal(false);
  // project id whose setup command is being edited
  const [editSetup, setEditSetup] = createSignal<string | null>(null);
  const [setupText, setSetupText] = createSignal("");

  /** asks which agent to open (or a plain terminal) and starts it in the worktree */
  const openWith = async (projectId: string, path: string) => {
    const a = await pickAgent();
    if (a === null) return;
    if (a === "shell") {
      const [program, args] = await defaultShell();
      await spawnSession(program, args, "shell", path, { project: projectId, worktree: path, kind: "shell" });
    } else {
      await launchDefaultAgent(projectId, path, a, AGENTS[a]);
    }
  };

  // clicking the main worktree of a project with nothing running opens the default agent right away,
  // ready to delegate with /orchestration
  const opening = new Set<string>();
  const openWorktree = async (projectId: string, path: string, isMain: boolean) => {
    selectWorktree(path);
    if (!isMain || opening.has(path)) return;
    await refreshSessions();
    if (sessionsOf(path).length > 0) return;
    opening.add(path);
    try {
      await launchDefaultAgent(projectId, path, defaultAgent(), AGENTS[defaultAgent()]);
      await refreshSessions();
    } catch (e) {
      setError(String(e));
    } finally {
      opening.delete(path);
    }
  };

  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
      setError("");
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
      // a new project opens straight into Claude on its main worktree
      const main = projects().find((p) => p.id === added.id)?.worktrees.find((w) => w.is_main);
      if (main) {
        selectWorktree(main.path);
        if (sessionsOf(main.path).length === 0) await openWith(added.id, main.path);
      }
    });

  const submitWorktree = (projectId: string) =>
    run(async () => {
      const name = newName().trim();
      setCreating(null);
      setNewName("");
      if (!name) return;
      const w = await createWorktree(projectId, name);
      selectWorktree(w.path);
      await openWith(projectId, w.path);
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
      await navigator.clipboard.writeText(text);
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
    { label: "Apagar worktree", icon: "🗑", danger: true, disabled: w.is_main, run: () => dropWorktree(p.id, w.path, label) },
  ];

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
    { label: "Tirar projeto da lista", icon: "✕", danger: true, run: () => run(() => removeProject(p.id)) },
  ];

  const dropWorktree = async (projectId: string, path: string, name: string) => {
    const ok = await ask(
      `Remover o worktree "${name}"?\n\nA pasta e o branch são apagados, incluindo alterações não guardadas. Os terminais dele são fechados.`,
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
              <button class="icon" title="Tirar da lista (não apaga ficheiros)" onClick={() => run(() => removeProject(p.id))}>
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
            <Show when={creating() === p.id}>
              <input
                class="wt-input"
                placeholder="nome do worktree"
                value={newName()}
                onInput={(e) => setNewName(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void submitWorktree(p.id);
                  if (e.key === "Escape") setCreating(null);
                }}
                ref={(el) => queueMicrotask(() => el.focus())}
              />
            </Show>
            <For each={[...p.worktrees].sort((a, b) => Number(b.is_main) - Number(a.is_main) || Number(isPinned(b.path)) - Number(isPinned(a.path)))}>
              {(w) => {
                const label = () => w.branch ?? "(detached)";
                const live = () => sessionsOf(w.path).filter((s) => s.running).length;
                const working = () => sessionsOf(w.path).some((s) => sessionState(s) === "working");
                const needs = () => attention().has(norm(w.path));
                return (
                  <div
                    class="item wt"
                    classList={{ active: norm(selectedWorktree()) === norm(w.path) }}
                    onClick={() => void openWorktree(p.id, w.path, w.is_main)}
                    onContextMenu={(e) => showMenu(e, worktreeMenu(p, w, label()))}
                    title={w.path}
                  >
                    <span
                      class="dot"
                      classList={{ on: live() > 0, work: working(), attn: needs() }}
                      title={needs() ? "O agente parou: precisa de atenção" : working() ? "A trabalhar" : live() > 0 ? "Parado" : ""}
                    />
                    <span class="name">{w.is_main ? (p.default_branch ?? "main") : label()}</span>
                    <Show when={w.is_main && w.branch !== (p.default_branch ?? "main")}>
                      <span class="off-main" title={`O worktree principal está em "${label()}", não em ${p.default_branch ?? "main"}`}>
                        ⚠
                      </span>
                    </Show>
                    <Show when={live() > 0}>
                      <span class="count">{live()}</span>
                    </Show>
                    <Show when={isPinned(w.path)}>
                      <span class="pin" title="Fixado">⚲</span>
                    </Show>
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
        ⏱ Automations
      </button>
      <div class="agent-pick" title="Agente aberto ao criar projetos e worktrees">
        <span>Agente</span>
        <div class="menu-wrap">
          <button class="menu-btn" onClick={() => setAgentMenu(!agentMenu())}>
            {defaultAgent()}
            <span class="chev">▾</span>
          </button>
          <Show when={agentMenu()}>
            <div class="menu-back" onClick={() => setAgentMenu(false)} />
            <div class="menu up">
              <For each={Object.keys(AGENTS)}>
                {(a) => (
                  <button
                    class="menu-item"
                    classList={{ on: a === defaultAgent() }}
                    onClick={() => {
                      setDefaultAgent(a);
                      setAgentMenu(false);
                    }}
                  >
                    {a}
                    <Show when={a === defaultAgent()}>
                      <span class="tick">✓</span>
                    </Show>
                  </button>
                )}
              </For>
            </div>
          </Show>
        </div>
      </div>
      <Show when={error()}>
        <div class="error">{error()}</div>
      </Show>
    </aside>
  );
}
