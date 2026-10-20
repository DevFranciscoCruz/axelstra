import { open, ask } from "@tauri-apps/plugin-dialog";
import { For, Show, createSignal } from "solid-js";
import { addProject, createWorktree, removeProject, removeWorktree, killSession, removeSession } from "./api";
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

export function Sidebar() {
  // project id currently showing the "new worktree" input
  const [creating, setCreating] = createSignal<string | null>(null);
  const [newName, setNewName] = createSignal("");

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
              <span class="pname">{p.name}</span>
              <button class="icon" title="Novo worktree" onClick={() => setCreating(creating() === p.id ? null : p.id)}>
                +
              </button>
              <button class="icon" title="Tirar da lista (não apaga ficheiros)" onClick={() => run(() => removeProject(p.id))}>
                ×
              </button>
            </div>
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

      <Show when={error()}>
        <div class="error">{error()}</div>
      </Show>
    </aside>
  );
}
