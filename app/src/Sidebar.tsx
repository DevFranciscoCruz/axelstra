import { open, ask } from "@tauri-apps/plugin-dialog";
import { For, Show, createResource, createSignal } from "solid-js";
import { addProject, projectLogo, createWorktree, setProjectSetup, defaultShell, spawnSession, writeSession, removeProject, removeWorktree, killSession, removeSession } from "./api";
import logo from "./assets/logo.png";
import {
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
  // project id whose setup command is being edited
  const [editSetup, setEditSetup] = createSignal<string | null>(null);
  const [setupText, setSetupText] = createSignal("");

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
      if (typeof dir === "string") await addProject(dir);
    });

  const submitWorktree = (projectId: string) =>
    run(async () => {
      const name = newName().trim();
      setCreating(null);
      setNewName("");
      if (!name) return;
      const w = await createWorktree(projectId, name);
      selectWorktree(w.path);
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
            <div class="project-head" title={p.path}>
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
            <For each={p.worktrees}>
              {(w) => {
                const label = () => w.branch ?? "(detached)";
                const live = () => sessionsOf(w.path).filter((s) => s.running).length;
                return (
                  <div
                    class="item wt"
                    classList={{ active: norm(selectedWorktree()) === norm(w.path) }}
                    onClick={() => selectWorktree(w.path)}
                    title={w.path}
                  >
                    <span class="dot" classList={{ on: live() > 0 }} />
                    <span class="name">{w.is_main ? `${label()} (principal)` : label()}</span>
                    <Show when={live() > 0}>
                      <span class="count">{live()}</span>
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
      <Show when={error()}>
        <div class="error">{error()}</div>
      </Show>
    </aside>
  );
}
