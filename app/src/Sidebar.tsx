import { open } from "@tauri-apps/plugin-dialog";
import { Icon } from "./Icon";
import { ask } from "./Confirm";
import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { addProject, getSettings, detectPackageManagers, projectLogo, createWorktree, launchDefaultAgent, openInEditor, setProjectSetup, defaultShell, spawnSession, writeSession, removeProject, removeWorktree, killSession, removeSession } from "./api";
import { Modal } from "./Modal";
import { MenuItem, showMenu, showMenuAt } from "./ContextMenu";
import { invoke } from "@tauri-apps/api/core";
import { press } from "./a11y";
import { S } from "./strings";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import logo from "./assets/logo.png";
import {
  AGENTS,
  agentKeys,
  isPinned,
  togglePin,
  setAttention,
  setShowAutomations,
  attention,
  sessionState,
  wtStatus,
  prByBranch,
  prKey,
  projectAgent,
  projectPrefsOf,
  projectTerse,
  setProjectPref,
  watchDispatch,
  dispatchTopics,
  confirmDiscard,
  editorDirty,
  flowState,
  setEditorDirty,
  norm,
  showNotice,
  setShowDispatcher,
  setShowSettings,
  projects,
  refreshProjects,
  refreshSessions,
  selectWorktree,
  selectedWorktree,
  sessions,
  sessionsOf,
  setError,
  activeSession,
  openSession,
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
  // titles the dispatcher gave each worktree (axelstra dispatch), polled by the store while the sidebar is open
  onCleanup(watchDispatch());
  const topic = (path: string) => dispatchTopics()[norm(path)];
  // worktrees whose agent list is folded away
  const AGENTS_KEY = "axelstra.agentsHidden";
  const [agentsHidden, setAgentsHidden] = createSignal<ReadonlySet<string>>(
    (() => {
      try {
        return new Set<string>(JSON.parse(localStorage.getItem(AGENTS_KEY) ?? "[]"));
      } catch {
        return new Set<string>();
      }
    })(),
  );
  const toggleAgents = (path: string) => {
    const next = new Set(agentsHidden());
    const k = norm(path);
    if (!next.delete(k)) next.add(k);
    setAgentsHidden(next);
    try {
      localStorage.setItem(AGENTS_KEY, JSON.stringify([...next]));
    } catch {
      /* preference only */
    }
  };
  // worktrees that only hold workers of an agent in another worktree: shown inside that worktree's box, not as rows
  const nested = createMemo(() => {
    const all = sessions();
    const byId = new Map(all.map((x) => [x.id, x]));
    const out = new Set<string>();
    const byWt = new Map<string, typeof all>();
    for (const x of all) {
      const k = norm(x.meta?.worktree);
      if (k) byWt.set(k, [...(byWt.get(k) ?? []), x]);
    }
    for (const [k, list] of byWt) {
      const inParent = (x: (typeof all)[number]) => {
        const par = x.meta?.parent !== undefined ? byId.get(x.meta.parent) : undefined;
        return !!x.meta?.worker && !!par && norm(par.meta?.worktree) !== k;
      };
      if (list.every(inParent)) out.add(k);
    }
    return out;
  });
  const idleLabel = (ms?: number) => {
    if (ms === undefined) return "";
    const m = Math.floor(ms / 60000);
    return m < 1 ? "agora" : m < 60 ? m + "m" : m < 1440 ? Math.floor(m / 60) + "h" : Math.floor(m / 1440) + "d";
  };
  const [editSetup, setEditSetup] = createSignal<string | null>(null);
  const [setupText, setSetupText] = createSignal("");
  // package managers installed on this machine, offered as one-click setup commands
  const [pms, setPms] = createSignal<{ installed: string[]; suggested: string | null }>({ installed: [], suggested: null });
  createEffect(() => {
    const id = editSetup();
    if (!id) return;
    detectPackageManagers(id).then(setPms, () => setPms({ installed: [], suggested: null }));
  });

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
      const dir = await open({ directory: true, multiple: false, title: "Escolha a pasta do projeto" });
      if (typeof dir !== "string") return;
      const added = await addProject(dir);
      await refreshProjects();
      const main = projects().find((p) => p.id === added.id)?.worktrees.find((w) => w.is_main);
      if (main) selectWorktree(main.path);
    });

  // a project with its own default agent starts new worktrees with it
  createEffect(() => {
    const id = creating();
    const own = id ? projectPrefsOf(id).agent : undefined;
    if (own && AGENTS[own]) setNewAgentSig(own);
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
      if (newAgent() !== "none") await launchDefaultAgent(projectId, w.path, newAgent(), AGENTS[newAgent()], undefined, undefined, projectTerse(projectId));
      let setup = projects().find((p) => p.id === projectId)?.setup;
      if (!setup && (await getSettings().then((x) => x.auto_install, () => false))) {
        const pm = await detectPackageManagers(projectId).catch(() => null);
        if (pm?.package_json && pm.suggested) setup = `${pm.suggested} install`;
      }
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
    { label: "Atualizar", icon: "refresh", run: async () => { await refreshProjects(); await refreshSessions(); } },
    {
      label: S.replies,
      icon: "sparkle",
      children: [
        { label: "Seguir o global", icon: projectTerse(p.id) ? undefined : "check", run: () => setProjectPref(p.id, { terse: undefined }) },
        ...(["off", "short", "tiny"] as const).map((v) => ({ label: S.terse[v].label, icon: projectTerse(p.id) === v ? "check" : undefined, run: () => setProjectPref(p.id, { terse: v }) })),
      ],
    },
    { separator: true },
    {
      label: "Abrir em",
      icon: "folder",
      children: [
        { label: "Explorador de ficheiros", run: () => run(() => revealItemInDir(w.path)) },
        { label: "VS Code", run: () => run(() => openInEditor(w.path, "code")) },
      ],
    },
    { label: "Copiar caminho", icon: "copy", run: () => copy(w.path) },
    { label: "Copiar nome do worktree", icon: "copy", run: () => copy(w.branch ?? label) },
    { separator: true },
    { label: isPinned(w.path) ? "Desafixar" : "Fixar", icon: "pin", run: () => togglePin(w.path) },
    { label: "Marcar como não lido", icon: "circleHalf", run: () => setAttention((cur) => new Set(cur).add(norm(w.path))) },
    { separator: true },
    { label: "Dormir", icon: "moon", run: () => sleepWorktree(w.path, label) },
    { label: "Remover worktree", icon: "x", danger: true, disabled: w.is_main, run: () => dropWorktree(p.id, w.path, label) },
  ];

  const dropProject = async (id: string, name: string) => {
    const ok = await ask(`Tirar "${name}" da lista?

Os ficheiros e worktrees não são apagados.`, { title: "Tirar projeto", kind: "warning", okLabel: "Tirar", cancelLabel: "Cancelar" });
    if (ok) await run(() => removeProject(id));
  };

  const projectMenu = (p: { id: string; name: string; path: string; setup?: string | null }): MenuItem[] => [
    { label: "Novo worktree", icon: "plus", run: () => setCreating(p.id) },
    { label: "Comando de setup…", icon: "gear", run: () => { setSetupText(p.setup ?? ""); setEditSetup(p.id); } },
    {
      label: "Agente por omissão",
      icon: "sparkle",
      children: [
        { label: "Seguir o global", icon: projectPrefsOf(p.id).agent ? undefined : "check", run: () => setProjectPref(p.id, { agent: undefined }) },
        ...agentKeys().map((a) => ({ label: a, icon: projectPrefsOf(p.id).agent === a ? "check" : undefined, run: () => setProjectPref(p.id, { agent: a }) })),
      ],
    },
    { separator: true },
    {
      label: "Abrir em",
      icon: "folder",
      children: [
        { label: "Explorador de ficheiros", run: () => run(() => revealItemInDir(p.path)) },
      ],
    },
    { label: "Copiar caminho", icon: "copy", run: () => copy(p.path) },
    { separator: true },
    { label: "Tirar projeto da lista", icon: "x", danger: true, run: () => dropProject(p.id, p.name) },
  ];

  // worktrees being removed: a second click (or menu entry) while it runs is ignored
  const removing = new Set<string>();
  const dropWorktree = async (projectId: string, path: string, name: string) => {
    if (removing.has(norm(path))) return showNotice(`A remover "${name}"…`);
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
    if (!ok || removing.has(norm(path))) return;
    removing.add(norm(path));
    showNotice(`A remover "${name}"…`);
    try {
      await run(async () => {
        for (const s of sessionsOf(path)) {
          if (s.running) await killSession(s.id);
          await removeSession(s.id);
        }
        await removeWorktree(projectId, path);
        if (norm(selectedWorktree()) === norm(path)) selectWorktree(null);
      });
      showNotice(`"${name}" removido.`);
    } finally {
      removing.delete(norm(path));
    }
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
              <button class="icon" title="Novo worktree" aria-label="Novo worktree" onClick={() => setCreating(creating() === p.id ? null : p.id)}>
                <Icon name="plus" />
              </button>
              <button
                class="icon"
                title={p.setup ? `Setup: ${p.setup}` : "Comando de setup para novos worktrees"} aria-label={p.setup ? `Setup: ${p.setup}` : "Comando de setup para novos worktrees"}
                onClick={() => {
                  setSetupText(p.setup ?? "");
                  setEditSetup(editSetup() === p.id ? null : p.id);
                }}
              >
                <Icon name="gear" />
              </button>
              <button class="icon" title="Tirar da lista (não apaga ficheiros)" aria-label="Tirar projeto da lista" onClick={() => void dropProject(p.id, p.name)}>
                <Icon name="x" />
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
              <Show when={pms().installed.length > 0}>
                <div class="pm-row">
                  <For each={pms().installed}>
                    {(m) => (
                      <button
                        class="pill"
                        classList={{ active: setupText() === `${m} install` }}
                        title={m === pms().suggested ? "Sugerido pelo lockfile do projeto" : `Usar ${m} install`}
                        onClick={() => setSetupText(`${m} install`)}
                      >
                        {m}
                        {m === pms().suggested ? <Icon name="star" size={10} inline /> : null}
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </Show>
            <Show when={pendingWt()?.project === p.id}>
              <div class="item wt pending">
                <span class="dot work" />
                <span class="name">A criar {pendingWt()!.name}…</span>
              </div>
            </Show>
            <For each={[...p.worktrees].filter((w) => !nested().has(norm(w.path))).sort((a, b) => Number(b.is_main) - Number(a.is_main) || Number(isPinned(b.path)) - Number(isPinned(a.path)))}>
              {(w) => {
                const label = () => w.branch ?? "(detached)";
                const live = () => sessionsOf(w.path).filter((s) => s.running).length;
                const working = () => sessionsOf(w.path).some((s) => sessionState(s) === "working");
                const waiting = () => sessionsOf(w.path).some((s) => sessionState(s) === "waiting");
                const needs = () => attention().has(norm(w.path));
                const queued = () => live() === 0 && flowState()[norm(w.path)] === "queued";
                const st = () => wtStatus()[norm(w.path)];
                const pr = () => prByBranch()[prKey(p.id, w.branch)];
                // agents of this worktree plus the workers its agents spawned (axelstration), even when they run in other worktrees
                const agents = () => {
                  const own = sessionsOf(w.path).filter((x) => x.meta?.kind === "agent");
                  const ids = new Set(own.map((x) => x.id));
                  const workers = sessions().filter((x) => x.meta?.worker && x.meta.kind === "agent" && x.meta.parent !== undefined && ids.has(x.meta.parent) && !ids.has(x.id));
                  return [...own, ...workers];
                };
                return (
                  <>
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
                    <Show when={!w.is_main && topic(w.path)?.title && topic(w.path)!.title !== label()}>
                      <span class="wt-topic" title={topic(w.path)!.title}>{topic(w.path)!.title}</span>
                    </Show>
                    <Show when={topic(w.path)?.archived}>
                      <span class="wt-badge" title="Arquivado: o despachante já não envia pedidos para aqui">arquivado</span>
                    </Show>
                    <Show when={w.is_main && w.branch !== (p.default_branch ?? "main")}>
                      <span class="off-main" title={`O worktree principal está em "${label()}", não em ${p.default_branch ?? "main"}`}><Icon name="warning" size={12} /></span>
                    </Show>
                    <Show when={st()?.dirty}>
                      <span class="wt-badge" title={`${st()!.dirty} ficheiro(s) com alterações por guardar`}><i class="bdot" />{st()!.dirty}</span>
                    </Show>
                    <Show when={st()?.ahead || st()?.behind}>
                      <span class="wt-badge" title={`${st()!.ahead} commit(s) à frente e ${st()!.behind} atrás do remoto`}>
                        {st()!.ahead ? <span class="bd"><Icon name="arrowUp" size={10} />{st()!.ahead}</span> : null}
                        {st()!.behind ? <span class="bd"><Icon name="arrowDown" size={10} />{st()!.behind}</span> : null}
                      </span>
                    </Show>
                    <Show when={pr()}>
                      <span
                        class="wt-badge pr"
                        classList={{ pass: pr()!.checks === "pass", fail: pr()!.checks === "fail", pend: pr()!.checks === "pending" }}
                        title={`PR #${pr()!.number}: ${pr()!.title}${pr()!.checks ? ` (CI: ${pr()!.checks})` : ""}`}
                      >
                        #{pr()!.number}
                        {pr()!.checks === "pass" ? <Icon name="check" size={10} inline /> : pr()!.checks === "fail" ? <Icon name="x" size={10} inline /> : pr()!.checks === "pending" ? <Icon name="ellipsis" size={10} inline /> : null}
                      </span>
                    </Show>
                    <Show when={live() > 0}>
                      <span class="count">{live()}</span>
                    </Show>
                    <Show when={isPinned(w.path)}>
                      <span class="pin" title="Fixado"><Icon name="pin" size={11} /></span>
                    </Show>
                    <button
                      class="x"
                      title="Mais ações" aria-label="Mais ações"
                      onClick={(e) => {
                        e.stopPropagation();
                        const r = e.currentTarget.getBoundingClientRect();
                        showMenuAt(r.left, r.bottom + 2, worktreeMenu(p, w, label()));
                      }}
                    >
                      <Icon name="more" />
                    </button>
                    <Show when={!w.is_main}>
                      <button
                        class="x"
                        title="Remover worktree" aria-label="Remover worktree"
                        onClick={(e) => {
                          e.stopPropagation();
                          void dropWorktree(p.id, w.path, label());
                        }}
                      >
                        <Icon name="x" />
                      </button>
                    </Show>
                  </div>
                  <Show when={agents().length > 0}>
                    <div class="wt-agents">
                      <button class="wt-agents-head" onClick={() => toggleAgents(w.path)} aria-expanded={!agentsHidden().has(norm(w.path))}>
                        <span>{agents().length} {agents().length === 1 ? "agente" : "agentes"}</span>
                        <Icon name={agentsHidden().has(norm(w.path)) ? "chevRight" : "chevDown"} size={11} />
                      </button>
                      <Show when={!agentsHidden().has(norm(w.path))}>
                        <For each={agents()}>
                          {(a) => (
                            <div
                              class="wt-agent"
                              classList={{ active: activeSession() === a.id && norm(selectedWorktree()) === norm(w.path) }}
                              {...press(() => openSession(a.meta?.worktree ?? w.path, a.id))}
                              title={a.label ?? `Sessão ${a.id}`}
                            >
                              <span class="dot" classList={{ on: a.running, work: sessionState(a) === "working", wait: sessionState(a) === "waiting" }} />
                              <span class="wt-agent-name">{a.meta?.worker ? <Icon name="subArrow" size={11} inline /> : null}{a.label ?? `Sessão ${a.id}`}</span>
                              <span class="wt-agent-time">{a.running ? idleLabel(a.idle_ms) : "fim"}</span>
                            </div>
                          )}
                        </For>
                      </Show>
                    </div>
                  </Show>
                  </>
                );
              }}
            </For>
          </div>
        )}
      </For>

      <div class="grow" />
      <button class="side-link" onClick={() => setShowAutomations(true)}>
        <Icon name="clock" size={13} /> Automatizações
      </button>
      <button class="side-link" onClick={() => setShowDispatcher(true)} title="Para onde foi cada pedido feito na main">
        <Icon name="branch" size={13} /> Despachante
      </button>
      <button class="side-link" onClick={() => setShowSettings(true)}>
        <Icon name="gear" size={13} /> Definições
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
                  <For each={agentKeys()}>{(a) => <option value={a}>{a}</option>}</For>
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
